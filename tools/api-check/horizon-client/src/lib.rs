//! Mock of horizon_client::mod_api (see ../adapter/Cargo.toml) for the hooks/helpers horizon_create uses, following
//! horizon-mod-api-gen's rules at Nightly 6560641 (events = hook params, `&mut scalar` by value,
//! replace -> returns Option, collect -> stays a field; Host = catalog helpers).
pub mod mod_api {
    use std::collections::BTreeMap;
    #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
    pub struct JavaString(pub Vec<u16>);
    impl From<&str> for JavaString { fn from(s: &str) -> Self { JavaString(s.encode_utf16().collect()) } }
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub enum Tag { End, Byte(i8), Short(i16), Int(i32), Long(i64), Float(u32), Double(u64), String(JavaString), Compound(Compound) }
    pub type Compound = BTreeMap<JavaString, Tag>;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct BlockStateId(pub u32);
    pub struct ItemStack;
    pub struct Text;
    impl Text { pub fn literal(_s: impl Into<String>) -> Self { Text } }
    #[derive(Default)]
    pub struct ModBlockEntityFields { pub id: String, pub nbt: Compound, pub items: Vec<Option<ItemStack>>, pub data: Vec<i32> }
    pub struct ModBlockEntity { fields: ModBlockEntityFields }
    impl std::ops::Deref for ModBlockEntity { type Target = ModBlockEntityFields; fn deref(&self) -> &Self::Target { &self.fields } }
    impl std::ops::DerefMut for ModBlockEntity { fn deref_mut(&mut self) -> &mut Self::Target { &mut self.fields } }
    pub struct BlockEntityDraw { pub partial: f32, pub game_time: i64, pub light: u32 }
    impl BlockEntityDraw {
        pub fn push(&mut self) {} pub fn pop(&mut self) {}
        pub fn translate(&mut self, _by: [f32; 3]) {}
        pub fn rotate_about(&mut self, _axis: [f32; 3], _degrees: f32, _pivot: [f32; 3]) {}
        pub fn scale(&mut self, _by: [f32; 3]) {} pub fn keep(&mut self) {}
        pub fn spin(&mut self, _axis: [f32; 3], _pivot: [f32; 3], _dpt: f32, _phase: f32) {}
        pub fn still(&mut self) {}
    }
    pub trait Host {
        fn block(&self, _pos: [i32; 3]) -> String { unimplemented!() }
        fn block_property(&self, _pos: [i32; 3], _key: &str) -> Option<String> { unimplemented!() }
        fn neighbor_signal(&mut self, _dimension: &str, _pos: [i32; 3]) -> i32 { unimplemented!() }
        fn set_block_property(&mut self, _pos: [i32; 3], _key: &str, _value: &str) -> bool { unimplemented!() }
        fn set_break_progress(&mut self, _breaker: i32, _pos: [i32; 3], _progress: i32) { unimplemented!() }
        fn destroy_block(&mut self, _pos: [i32; 3], _tool: Option<&ItemStack>, _collect: bool) -> Option<Vec<ItemStack>> { unimplemented!() }
        fn block_state_property(&self, _state: BlockStateId, _key: &str) -> Option<&str> { unimplemented!() }
        fn draw_model(&self, _draw: &mut BlockEntityDraw, _model: &str, _tint: u32) -> bool { unimplemented!() }
        fn draw_text(&self, _draw: &mut BlockEntityDraw, _text: &Text, _at: [f32; 2], _color: u32, _glowing: bool, _outline: u32) -> f32 { unimplemented!() }
    }
    #[non_exhaustive] pub struct StandaloneModelsEvent<'a> { pub models: &'a mut Vec<String> }
    #[non_exhaustive] pub struct BlockEntityEvent<'a> { pub block: &'a str }
    #[non_exhaustive] pub struct BlockEntityTicksEvent<'a> { pub block: &'a str, pub ticks: bool }
    #[non_exhaustive] pub struct LevelTickPreEvent<'a> { pub dimension: &'a str }
    #[non_exhaustive] pub struct LevelTickPostEvent<'a> { pub dimension: &'a str }
    #[non_exhaustive] pub struct BlockEntityTickEvent<'a> { pub pos: [i32; 3], pub entity: &'a mut ModBlockEntity }
    #[non_exhaustive] pub struct BlockEntityUpdateTagEvent<'a> { pub pos: [i32; 3], pub entity: &'a ModBlockEntity }
    #[non_exhaustive] pub struct BlockEntityRenderBoundingBoxEvent<'a> { pub pos: [i32; 3], pub block: &'a str, pub state: BlockStateId, pub entity: &'a ModBlockEntity, pub bounds: &'a mut [f32; 6] }
    #[non_exhaustive] pub struct BlockEntitySubmitEvent<'a> { pub pos: [i32; 3], pub block: &'a str, pub state: BlockStateId, pub entity: &'a ModBlockEntity, pub draw: &'a mut BlockEntityDraw }
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NativeMod { pub id: String, pub title: String, pub description: String, pub keys: Vec<[String; 3]> }
    #[non_exhaustive] pub struct ModsEvent<'a> { pub mods: &'a mut Vec<NativeMod> }
    pub trait Hooks: Send + Sync {
        fn mods(&self, _host: &mut dyn Host, _event: &mut ModsEvent<'_>) {}
        fn standalone_models(&self, _event: &mut StandaloneModelsEvent<'_>) {}
        fn block_entity(&self, _host: &mut dyn Host, _event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> { None }
        fn block_entity_ticks(&self, _host: &dyn Host, _event: &mut BlockEntityTicksEvent<'_>) {}
        fn level_tick_pre(&self, _host: &mut dyn Host, _event: &mut LevelTickPreEvent<'_>) {}
        fn level_tick_post(&self, _host: &mut dyn Host, _event: &mut LevelTickPostEvent<'_>) {}
        fn block_entity_tick(&self, _host: &mut dyn Host, _event: &mut BlockEntityTickEvent<'_>) {}
        fn block_entity_update_tag(&self, _event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> { None }
        fn block_entity_render_bounding_box(&self, _host: &dyn Host, _event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {}
        fn block_entity_submit(&self, _host: &dyn Host, _event: &mut BlockEntitySubmitEvent<'_>) {}
    }
}
