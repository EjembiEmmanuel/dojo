use std::collections::{HashMap, HashSet};

use crate::class::ClassHash;
use crate::contract::ContractAddress;
use crate::event::OrderedEvent;
use crate::message::OrderedL2ToL1Message;
use crate::FieldElement;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TxExecInfo {
    /// Transaction validation call info; [None] for `L1Handler`.
    pub validate_call_info: Option<CallInfo>,
    /// Transaction execution call info; [None] for `Declare`.
    pub execute_call_info: Option<CallInfo>,
    /// Fee transfer call info; [None] for `L1Handler`.
    pub fee_transfer_call_info: Option<CallInfo>,
    /// The actual fee that was charged (in Wei).
    pub actual_fee: u128,
    /// Actual execution resources the transaction is charged for,
    /// including L1 gas and additional OS resources estimation.
    pub actual_resources: HashMap<String, u64>,
    /// Error string for reverted transactions; [None] if transaction execution was successful.
    pub revert_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExecutionResources {
    pub n_steps: u64,
    pub n_memory_holes: u64,
    pub builtin_instance_counter: HashMap<String, u64>,
}

/// The call type.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CallType {
    #[default]
    /// Normal contract call.
    Call,
    /// Library call.
    Delegate,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EntryPointType {
    #[default]
    External,
    L1Handler,
    Constructor,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CallInfo {
    /// The contract address which the call is initiated from.
    pub caller_address: ContractAddress,
    /// The call type.
    pub call_type: CallType,
    /// The contract address.
    ///
    /// The contract address of the current call execution context. This would be the address of
    /// the contract whose code is currently being executed, or in the case of library call, the
    /// address of the contract where the library call is being initiated from.
    pub contract_address: ContractAddress,
    /// The address where the code is being executed.
    /// Optional, since there is no address to the code implementation in a delegate call.
    pub code_address: Option<ContractAddress>,
    /// The class hash, not given if it can be deduced from the storage address.
    pub class_hash: Option<ClassHash>,
    /// The entry point selector.
    pub entry_point_selector: FieldElement,
    /// The entry point type.
    pub entry_point_type: EntryPointType,
    /// The data used as the input to the execute entry point.
    pub calldata: Vec<FieldElement>,
    /// The data returned by the entry point execution.
    pub retdata: Vec<FieldElement>,
    /// The resources used by the execution.
    pub execution_resources: ExecutionResources,
    /// The list of ordered events generated by the execution.
    pub events: Vec<OrderedEvent>,
    /// The list of ordered l2 to l1 messages generated by the execution.
    pub l2_to_l1_messages: Vec<OrderedL2ToL1Message>,
    /// The list of storage addresses being read during the execution.
    pub storage_read_values: Vec<FieldElement>,
    /// The list of storage addresses being accessed during the execution.
    pub accessed_storage_keys: HashSet<FieldElement>,
    /// The list of inner calls triggered by the current call.
    pub inner_calls: Vec<CallInfo>,
    /// The total gas consumed by the call.
    pub gas_consumed: u128,
    /// True if the execution has failed, false otherwise.
    pub failed: bool,
}
