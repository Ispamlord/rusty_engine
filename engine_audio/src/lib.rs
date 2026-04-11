use bevy_ecs::prelude::*;
use kira::backend::cpal::CpalBackend;
use kira::{AudioManager, AudioManagerSettings};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioEvent {
    PlaySfx { id: String },
    StopAll,
}

#[derive(Resource, Default)]
pub struct AudioState {
    pub queued_events: Vec<AudioEvent>,
}

pub struct AudioRuntime {
    manager: Option<AudioManager<CpalBackend>>,
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("audio backend initialization failed: {0}")]
    Init(String),
}

impl AudioRuntime {
    pub fn new() -> Result<Self, AudioError> {
        let manager = AudioManager::<CpalBackend>::new(AudioManagerSettings::default())
            .map_err(|err| AudioError::Init(err.to_string()))?;

        Ok(Self {
            manager: Some(manager),
        })
    }

    pub fn silent() -> Self {
        Self { manager: None }
    }

    pub fn has_backend(&self) -> bool {
        self.manager.is_some()
    }
}

pub fn audio_sync_system(mut state: ResMut<AudioState>) {
    state.queued_events.clear();
}
