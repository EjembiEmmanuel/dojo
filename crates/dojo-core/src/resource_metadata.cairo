//! ResourceMetadata model.
//!
//! Manually expand to ensure that dojo-core
//! does not depend on dojo plugin to be built.
//!
const RESOURCE_METADATA_MODEL: felt252 = 'ResourceMetadata';

#[derive(Drop, Serde, PartialEq, Clone)]
struct ResourceMetadata {
    // #[key]
    resource_id: felt252,
    // #[capacity(10)].
    metadata_uri: Span<felt252>,
}

impl ResourceMetadataModel of dojo::model::Model<ResourceMetadata> {
    #[inline(always)]
    fn name(self: @ResourceMetadata) -> felt252 {
        'ResourceMetadata'
    }

    #[inline(always)]
    fn keys(self: @ResourceMetadata) -> Span<felt252> {
        let mut serialized = core::array::ArrayTrait::new();
        core::array::ArrayTrait::append(ref serialized, *self.resource_id);
        core::array::ArrayTrait::span(@serialized)
    }

    #[inline(always)]
    fn values(self: @ResourceMetadata) -> Span<felt252> {
        let mut serialized = core::array::ArrayTrait::new();
        core::serde::Serde::serialize(self.metadata_uri, ref serialized);
        core::array::ArrayTrait::span(@serialized)
    }

    #[inline(always)]
    fn layout(self: @ResourceMetadata) -> Span<u8> {
        let mut layout = core::array::ArrayTrait::new();
        dojo::database::introspect::Introspect::<ResourceMetadata>::layout(ref layout);
        core::array::ArrayTrait::span(@layout)
    }

    #[inline(always)]
    fn packed_size(self: @ResourceMetadata) -> usize {
        let mut layout = self.layout();
        dojo::packing::calculate_packed_size(ref layout)
    }
}

impl ResourceMetadataIntrospect<> of dojo::database::introspect::Introspect<ResourceMetadata<>> {
    #[inline(always)]
    fn size() -> usize {
        // len (1 felt) + capacity of 10 felts.
        11
    }

    #[inline(always)]
    fn layout(ref layout: Array<u8>) {
        // len
        layout.append(251);
        // 10 capacity.
        layout.append(251);
        layout.append(251);
        layout.append(251);
        layout.append(251);
        layout.append(251);
        layout.append(251);
        layout.append(251);
        layout.append(251);
        layout.append(251);
        layout.append(251);
    }

    #[inline(always)]
    fn ty() -> dojo::database::introspect::Ty {
        dojo::database::introspect::Ty::Struct(dojo::database::introspect::Struct {
            name: 'ResourceMetadata',
            attrs: array![].span(),
            children: array![dojo::database::introspect::serialize_member(@dojo::database::introspect::Member {
                name: 'resource_id',
                ty: dojo::database::introspect::Ty::Primitive('felt252'),
                attrs: array!['key'].span()
            }), dojo::database::introspect::serialize_member(@dojo::database::introspect::Member {
                name: 'metadata_uri',
                ty: dojo::database::introspect::Ty::Array(10),
                attrs: array![].span()
            })].span()
        })
    }
}

#[starknet::contract]
mod resource_metadata {
    use super::ResourceMetadata;

    #[storage]
    struct Storage {}

    #[external(v0)]
    fn name(self: @ContractState) -> felt252 {
        'ResourceMetadata'
    }

    #[external(v0)]
    fn unpacked_size(self: @ContractState) -> usize {
        dojo::database::introspect::Introspect::<ResourceMetadata>::size()
    }

    #[external(v0)]
    fn packed_size(self: @ContractState) -> usize {
        let mut layout = core::array::ArrayTrait::new();
        dojo::database::introspect::Introspect::<ResourceMetadata>::layout(ref layout);
        let mut layout_span = layout.span();
        dojo::packing::calculate_packed_size(ref layout_span)
    }

    #[external(v0)]
    fn layout(self: @ContractState) -> Span<u8> {
        let mut layout = core::array::ArrayTrait::new();
        dojo::database::introspect::Introspect::<ResourceMetadata>::layout(ref layout);
        core::array::ArrayTrait::span(@layout)
    }

    #[external(v0)]
    fn schema(self: @ContractState) -> dojo::database::introspect::Ty {
        dojo::database::introspect::Introspect::<ResourceMetadata>::ty()
    }

    #[external(v0)]
    fn ensure_abi(self: @ContractState, model: ResourceMetadata) {
    }
}
