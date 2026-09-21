use std::collections::BTreeMap;

use crate::devices::State;
use crate::platform;
use crate::AudioError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileAvailability {
    Unknown,
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardProfile {
    pub name: String,
    pub description: String,
    pub availability: ProfileAvailability,
    pub priority: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub driver: Option<String>,
    pub active_profile: Option<String>,
    pub profiles: Vec<CardProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sink {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub state: State,
    pub monitor_source_index: Option<u32>,
    pub card_index: Option<u32>,
    pub active_port: Option<String>,
    pub properties: BTreeMap<String, String>,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub state: State,
    pub monitor_of_sink_index: Option<u32>,
    pub card_index: Option<u32>,
    pub active_port: Option<String>,
    pub properties: BTreeMap<String, String>,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceOutput {
    pub index: u32,
    pub name: String,
    pub source_index: u32,
    pub client_index: Option<u32>,
    pub corked: bool,
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFacts {
    pub name: Option<String>,
    pub version: Option<String>,
    pub incarnation: u32,
    pub default_sink: Option<String>,
    pub default_source: Option<String>,
}

pub fn list_cards() -> Result<Vec<Card>, AudioError> {
    platform::list_cards()
}

pub fn set_card_profile(card: &str, profile: &str) -> Result<(), AudioError> {
    platform::set_card_profile(card, profile)
}

pub fn list_sinks() -> Result<Vec<Sink>, AudioError> {
    platform::list_sinks()
}

pub fn list_sources() -> Result<Vec<Source>, AudioError> {
    platform::list_sources()
}

pub fn list_source_outputs() -> Result<Vec<SourceOutput>, AudioError> {
    platform::list_source_outputs()
}

pub fn suspend_sink(name: &str, suspended: bool) -> Result<(), AudioError> {
    platform::suspend_sink(name, suspended)
}

pub fn set_default_sink(name: &str) -> Result<(), AudioError> {
    platform::set_default_sink(name)
}

pub fn server_facts() -> Result<ServerFacts, AudioError> {
    platform::server_facts()
}
