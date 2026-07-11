//! Audio runtime integration.
//!
//! Currently the runtime queues audio events each frame but does not yet
//! translate those events into mixer commands. Use [`AudioRuntime::silent`]
//! for headless or CI environments where CPAL cannot open a device.

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

impl AudioState {
    /// Queue an audio event to be consumed by the audio runtime.
    pub fn queue(&mut self, event: AudioEvent) {
        self.queued_events.push(event);
    }

    /// Remove all queued events without processing them.
    ///
    /// This is a temporary stub; once playback is wired up the events will be
    /// translated into kira mixer commands instead of being discarded.
    pub fn clear(&mut self) {
        self.queued_events.clear();
    }
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
    /// Try to initialize the CPAL audio backend.
    ///
    /// Returns an error if no audio device is available. Callers that must not
    /// fail should fall back to [`AudioRuntime::silent`].
    pub fn new() -> Result<Self, AudioError> {
        let manager = AudioManager::<CpalBackend>::new(AudioManagerSettings::default())
            .map_err(|err| AudioError::Init(err.to_string()))?;

        Ok(Self {
            manager: Some(manager),
        })
    }

    /// Create a headless runtime that accepts events but produces no output.
    pub fn silent() -> Self {
        Self { manager: None }
    }

    pub fn has_backend(&self) -> bool {
        self.manager.is_some()
    }
}

impl Default for AudioRuntime {
    fn default() -> Self {
        // Default to silent so that tests and headless callers do not require
        // an audio device. EngineApp explicitly tries `new()` first.
        Self::silent()
    }
}

pub fn audio_sync_system(mut state: ResMut<AudioState>) {
    // Queued events are drained here. Once playback is wired up they should be
    // translated into kira mixer commands instead of being discarded.
    state.queued_events.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_state_queues_events() {
        let mut state = AudioState::default();
        state.queue(AudioEvent::PlaySfx {
            id: "explosion".into(),
        });
        state.queue(AudioEvent::StopAll);
        assert_eq!(state.queued_events.len(), 2);
    }

    #[test]
    fn audio_state_clear_empties_queue() {
        let mut state = AudioState::default();
        state.queue(AudioEvent::StopAll);
        state.clear();
        assert!(state.queued_events.is_empty());
    }

    #[test]
    fn audio_system_clears_queued_events() {
        let mut world = World::new();
        world.insert_resource(AudioState::default());
        world
            .get_resource_mut::<AudioState>()
            .unwrap()
            .queue(AudioEvent::StopAll);

        let mut schedule = Schedule::default();
        schedule.add_systems(audio_sync_system);
        schedule.run(&mut world);

        assert!(world
            .get_resource::<AudioState>()
            .unwrap()
            .queued_events
            .is_empty());
    }

    #[test]
    fn silent_runtime_has_no_backend() {
        let runtime = AudioRuntime::silent();
        assert!(!runtime.has_backend());
    }

    #[test]
    fn default_runtime_is_silent() {
        let runtime = AudioRuntime::default();
        assert!(!runtime.has_backend());
    }
}
