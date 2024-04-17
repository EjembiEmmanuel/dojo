use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;

use anyhow::Context;
use katana_primitives::contract::ContractAddress;
use katana_primitives::genesis::json;
use libmdbx::DatabaseFlags;
use tempfile::NamedTempFile;

use crate::codecs::{Compress, Encode};
use crate::error::DatabaseError;
use crate::mdbx::DbEnv;
use crate::models::list::BlockList;
use crate::models::storage::ContractStorageKey;
use crate::tables::v0::StorageEntryChangeList;
use crate::tables::{Key, Table};
use crate::version::{
    create_db_version_file, get_db_version, remove_db_version_file, DatabaseVersionError,
};
use crate::{open_db_with_schema, tables, CURRENT_DB_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum DatabaseMigrationError {
    #[error("Unsupported database version for migration: {0}")]
    UnsupportedVersion(u32),

    #[error(transparent)]
    DatabaseVersion(#[from] DatabaseVersionError),

    #[error(transparent)]
    Database(#[from] DatabaseError),

    #[error("failed to open temporary file: {0}")]
    Io(#[from] io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Performs a database migration for an already initialized database with an older
/// version of the database schema.
///
/// Database migration can only be done on a supported older version of the database schema,
/// meaning not all older versions can be migrated from.
pub fn migrate_db<P: AsRef<Path>>(path: P) -> Result<(), DatabaseMigrationError> {
    // check that the db version is supported
    let ver = get_db_version(&path)?;

    match ver {
        0 => migrate_from_v0_to_v1(open_db_with_schema(&path)?)?,
        _ => {
            return Err(DatabaseMigrationError::UnsupportedVersion(ver));
        }
    }

    // Update the db version to the migrated version
    {
        // we have to remove it first because the version file is read-only
        remove_db_version_file(&path)?;
        create_db_version_file(path, CURRENT_DB_VERSION)
    }
    .context("Updating database version file")?;

    Ok(())
}

/// Perform migration for database version 0 to version 1.
///
/// # Changelog from v0 to v1
///
/// 1. [ContractClassChanges](tables::v0::ContractClassChanges)
/// - Renamed to [ClassChangeHistory](tables::ClassCh
///
/// 2. [StorageChanges](tables::v0::StorageChanges)
/// - Renamed to [StorageChangeHistory](tables::StorageChangeHistory)
///
/// 3. [NonceChanges](tables::v0::NonceChanges)
/// - Renamed to [NonceChangeHistory](tables::NonceChangeHistory)
///
/// 4. [StorageChangeSet](tables::v0::StorageChangeSet)
/// - Changed table type from dupsort to normal table.
/// - Changed key type to [ContractStorageKey](crate::models::storage::ContractStorageKey).
/// - Changed value type to [BlockList](crate::models::list::BlockList).
///
fn migrate_from_v0_to_v1(env: DbEnv<tables::v0::SchemaV0>) -> Result<(), DatabaseMigrationError> {
    // env.create_tables_from_schema::<tables::SchemaV1>()?;

    macro_rules! create_table {
        ($tx:expr, $table:ty, $flags:expr) => {
            $tx.inner.create_db(Some(<$table as Table>::NAME), $flags).map_err(|error| {
                DatabaseError::CreateTable { table: <$table as Table>::NAME, error }
            })?;
        };
    }

    env.update(|tx| {
        {
            let mut cursor = tx.cursor::<tables::v0::StorageChangeSet>()?;
            let mut old_entries: HashMap<ContractAddress, Vec<StorageEntryChangeList>> =
                HashMap::new();

            cursor.walk(None)?.enumerate().try_for_each(|(i, entry)| {
                let (key, val) = entry?;
                old_entries.entry(key).or_default().push(val);
                Result::<(), DatabaseError>::Ok(())
            })?;

            drop(cursor);
            unsafe {
                tx.drop_table::<tables::v0::StorageChangeSet>()?;
            }
            create_table!(tx, tables::StorageChangeSet, DatabaseFlags::default());

            for (key, vals) in old_entries {
                for val in vals {
                    let key = ContractStorageKey { contract_address: key, key: val.key };
                    let val = BlockList::from_iter(val.block_list);
                    tx.put::<tables::StorageChangeSet>(key, val)?;
                }
            }

            // move data from `NonceChanges` to `NonceChangeHistory`
            create_table!(tx, tables::NonceChangeHistory, DatabaseFlags::DUP_SORT);
            let mut cursor = tx.cursor::<tables::v0::NonceChanges>()?;
            cursor.walk(None)?.try_for_each(|entry| {
                let (key, val) = entry?;
                tx.put_unchecked::<tables::NonceChangeHistory>(key, val)?;
                Result::<(), DatabaseError>::Ok(())
            })?;

            create_table!(tx, tables::StorageChangeHistory, DatabaseFlags::DUP_SORT);
            // move data from `StorageChanges` to `StorageChangeHistory`
            let mut cursor = tx.cursor::<tables::v0::StorageChanges>()?;
            cursor.walk(None)?.try_for_each(|entry| {
                let (key, val) = entry?;
                tx.put_unchecked::<tables::StorageChangeHistory>(key, val)?;
                Result::<(), DatabaseError>::Ok(())
            })?;

            create_table!(tx, tables::ClassChangeHistory, DatabaseFlags::DUP_SORT);
            // move data from `ContractClassChanges` to `ClassChangeHistory`
            let mut cursor = tx.cursor::<tables::v0::ContractClassChanges>()?;
            cursor.walk(None)?.try_for_each(|entry| {
                let (key, val) = entry?;
                tx.put_unchecked::<tables::ClassChangeHistory>(key, val)?;
                Result::<(), DatabaseError>::Ok(())
            })?;
        }

        // drop the old tables
        unsafe {
            tx.drop_table::<tables::v0::NonceChanges>()?;
            tx.drop_table::<tables::v0::StorageChanges>()?;
            tx.drop_table::<tables::v0::ContractClassChanges>()?;
        }

        Ok(())
    })?
}

// - collects all the buffer that we want to push to a temp file
// - each yeeter will be speciifc to a particular table
// - when the buffer is full, we write to a temp file

struct Yeeter<T: Table> {
    files: Vec<YeeterFile>,
    buffer: Vec<(<T::Key as Encode>::Encoded, <T::Value as Compress>::Compressed)>,
}

impl<T: Table> Yeeter<T> {
    fn push(&mut self, key: T::Key, value: T::Value) {
        self.buffer.push((key.encode(), value.compress()));
    }

    fn flush(&mut self) -> Result<(), io::Error> {
        let mut buffer = Vec::with_capacity(self.buffer.len());
        std::mem::swap(&mut buffer, &mut self.buffer);

        // write to file
        let mut file = YeeterFile::new()?;
        for (key, value) in buffer {
            file.write(key.as_ref(), value.as_ref())?;
        }

        self.files.push(file);
        Ok(())
    }
}

struct YeeterFile {
    // the underlying file used to store the buffer
    file: NamedTempFile,
    // the total number of key/value pairs written to the file
    len: usize,
}

impl YeeterFile {
    fn new() -> Result<Self, io::Error> {
        let file = NamedTempFile::new()?;
        Ok(Self { file, len: 0 })
    }

    fn write(&mut self, key: &[u8], value: &[u8]) -> Result<(), io::Error> {
        let key_size = key.len().to_be_bytes();
        let value_size = value.len().to_be_bytes();

        self.file.write_all(&key_size)?;
        self.file.write_all(&value_size)?;
        self.file.write_all(&key)?;
        self.file.write_all(&value)?;

        self.len += 1;

        Ok(())
    }

    fn read_next(&mut self) -> Result<Option<(Vec<u8>, Vec<u8>)>, io::Error> {
        // check if we have reached the end of the file
        if self.len == 0 {
            return Ok(None);
        }

        // get thee sizes of the key and value
        let mut key_size = [0u8; 8];
        let mut value_size = [0u8; 8];
        self.file.read_exact(&mut key_size)?;
        self.file.read_exact(&mut value_size)?;

        // read the key and value
        let mut key = Vec::with_capacity(u64::from_be_bytes(key_size) as usize);
        let mut value = Vec::with_capacity(u64::from_be_bytes(value_size) as usize);
        self.file.read_exact(&mut key)?;
        self.file.read_exact(&mut value)?;

        self.len -= 1;

        Ok(Some((key, value)))
    }
}

#[cfg(test)]
mod tests {

    use std::path::PathBuf;

    use starknet::macros::felt;

    use super::migrate_db;
    use crate::mdbx::DbEnv;
    use crate::models::contract::{ContractClassChange, ContractNonceChange};
    use crate::models::list::BlockList;
    use crate::models::storage::{ContractStorageEntry, ContractStorageKey};
    use crate::tables::v0::{self, StorageEntryChangeList};
    use crate::{init_db, open_db, tables};

    const ERROR_CREATE_TEMP_DIR: &str = "Failed to create temp dir.";
    const ERROR_MIGRATE_DB: &str = "Failed to migrate db.";
    const ERROR_INIT_DB: &str = "Failed to initialize db.";

    fn create_test_db() -> (DbEnv, PathBuf) {
        let path = tempfile::TempDir::new().expect(ERROR_CREATE_TEMP_DIR).into_path();
        let db = init_db(&path).expect(ERROR_INIT_DB);
        (db, path)
    }

    // TODO(kariy): create Arbitrary for database key/value types to easily create random test vectors
    fn create_v0_test_db() -> (DbEnv<v0::SchemaV0>, PathBuf) {
        let path = tempfile::TempDir::new().expect(ERROR_CREATE_TEMP_DIR).into_path();
        let db = crate::init_db_with_schema::<v0::SchemaV0>(&path).expect(ERROR_INIT_DB);

        db.update(|tx| {
            let val1 = ContractNonceChange::new(felt!("0x1").into(), felt!("0x2"));
            let val2 = ContractNonceChange::new(felt!("0x2").into(), felt!("0x2"));
            let val3 = ContractNonceChange::new(felt!("0x3").into(), felt!("0x2"));
            tx.put::<v0::NonceChanges>(1, val1).unwrap();
            tx.put::<v0::NonceChanges>(1, val2).unwrap();
            tx.put::<v0::NonceChanges>(3, val3).unwrap();

            let val1 = ContractClassChange::new(felt!("0x1").into(), felt!("0x1"));
            let val2 = ContractClassChange::new(felt!("0x2").into(), felt!("0x1"));
            tx.put::<v0::ContractClassChanges>(1, val1).unwrap();
            tx.put::<v0::ContractClassChanges>(1, val2).unwrap();

            let val1 = StorageEntryChangeList::new(felt!("0x1"), vec![1, 2]);
            let val2 = StorageEntryChangeList::new(felt!("0x2"), vec![1, 3]);
            let val3 = StorageEntryChangeList::new(felt!("0x3"), vec![4, 5]);
            tx.put::<v0::StorageChangeSet>(felt!("0x1").into(), val1).unwrap();
            tx.put::<v0::StorageChangeSet>(felt!("0x1").into(), val2).unwrap();
            tx.put::<v0::StorageChangeSet>(felt!("0x2").into(), val3).unwrap();

            let subkey = ContractStorageKey::new(felt!("0x1").into(), felt!("0x1"));
            let val1 = ContractStorageEntry::new(subkey, felt!("0x1"));
            let subkey = ContractStorageKey::new(felt!("0x1").into(), felt!("0x2"));
            let val2 = ContractStorageEntry::new(subkey, felt!("0x2"));
            tx.put::<v0::StorageChanges>(1, val1).unwrap();
            tx.put::<v0::StorageChanges>(3, val2).unwrap();
        })
        .expect(ERROR_INIT_DB);

        (db, path)
    }

    #[test]
    fn migrate_from_current_version() {
        let (_, path) = create_test_db();
        assert_eq!(
            migrate_db(path).unwrap_err().to_string(),
            "Unsupported database version for migration: 1",
            "Can't migrate from the current version"
        );
    }

    #[test]
    fn migrate_from_v0() {
        // we cant have multiple instances of the db open in the same process, so we drop here first before migrating
        let (_, path) = create_v0_test_db();
        let _ = migrate_db(&path).expect(ERROR_MIGRATE_DB);
        let env = open_db(path).unwrap();

        env.view(|tx| {
            let mut cursor = tx.cursor::<tables::NonceChangeHistory>().unwrap();
            let val1 = cursor.seek_by_key_subkey(1, felt!("0x1").into()).unwrap();
            let val2 = cursor.seek_by_key_subkey(1, felt!("0x2").into()).unwrap();
            let val3 = cursor.seek_by_key_subkey(3, felt!("0x3").into()).unwrap();

            let exp_val1 = ContractNonceChange::new(felt!("0x1").into(), felt!("0x2"));
            let exp_val2 = ContractNonceChange::new(felt!("0x2").into(), felt!("0x2"));
            let exp_val3 = ContractNonceChange::new(felt!("0x3").into(), felt!("0x2"));
            assert_eq!(val1, Some(exp_val1));
            assert_eq!(val2, Some(exp_val2));
            assert_eq!(val3, Some(exp_val3));

            let mut cursor = tx.cursor::<tables::ClassChangeHistory>().unwrap();
            let val1 = cursor.seek_by_key_subkey(1, felt!("0x1").into()).unwrap();
            let val2 = cursor.seek_by_key_subkey(1, felt!("0x2").into()).unwrap();

            let exp_val1 = ContractClassChange::new(felt!("0x1").into(), felt!("0x1"));
            let exp_val2 = ContractClassChange::new(felt!("0x2").into(), felt!("0x1"));
            assert_eq!(val1, Some(exp_val1));
            assert_eq!(val2, Some(exp_val2));

            let key1 = ContractStorageKey::new(felt!("0x1").into(), felt!("0x1"));
            let key2 = ContractStorageKey::new(felt!("0x1").into(), felt!("0x2"));
            let key3 = ContractStorageKey::new(felt!("0x2").into(), felt!("0x3"));
            let val1 = tx.get::<tables::StorageChangeSet>(key1).unwrap();
            let val2 = tx.get::<tables::StorageChangeSet>(key2).unwrap();
            let val3 = tx.get::<tables::StorageChangeSet>(key3).unwrap();

            let exp_val1 = BlockList::from([1, 2]);
            let exp_val2 = BlockList::from([1, 3]);
            let exp_val3 = BlockList::from([4, 5]);
            assert_eq!(val1, Some(exp_val1));
            assert_eq!(val2, Some(exp_val2));
            assert_eq!(val3, Some(exp_val3));
        })
        .unwrap();
    }
}