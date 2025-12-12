pub mod plugin_store;
pub mod task_runner;

use crate::plugins::MenuItem as PluginMenuItem;
use anyhow::Result;

pub trait MenuProvider: Send + Sync {
    fn menu_items(&self) -> Vec<PluginMenuItem>;
    fn handle_event(&self, event_id: &str) -> Result<()>;
}

pub struct FeatureRegistry {
    features: Vec<Box<dyn MenuProvider>>,
}

impl FeatureRegistry {
    pub fn new() -> Self {
        Self {
            features: Vec::new(),
        }
    }

    pub fn register(&mut self, feature: Box<dyn MenuProvider>) {
        self.features.push(feature);
    }

    pub fn features(&self) -> &[Box<dyn MenuProvider>] {
        &self.features
    }
}

impl Default for FeatureRegistry {
    fn default() -> Self {
        Self::new()
    }
}
