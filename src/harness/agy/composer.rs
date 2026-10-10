//! agy composer policy stub. L4 replaces the read with the captured grammar.
use crate::harness::{adapter::ComposerPolicy, composer::ComposerRead, recipe::PokeCapabilities};

pub(super) struct AgyComposer;
impl ComposerPolicy for AgyComposer {
    fn capabilities(&self, _: Option<&str>) -> PokeCapabilities {
        PokeCapabilities::NONE
    }
    fn read(&self, _: &str, _: Option<u16>) -> ComposerRead {
        ComposerRead::Unreadable
    }
    fn clear_key(&self) -> &'static str {
        ""
    }
    fn restore_text(&self, _: &str) -> String {
        String::new()
    }
}
