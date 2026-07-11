//! Physics runtime integration.
//!
//! The [`PhysicsWorld`] resource is allocated and stepped each frame, but the
//! actual Rapier integration pipeline is not yet wired up. `step()` currently
//! advances the internal counter only.

use bevy_ecs::prelude::*;
use rapier2d::na::Vector2;
use rapier2d::prelude::{ColliderSet, RigidBodySet};

/// Default gravity vector pointing down (negative Y).
pub const DEFAULT_GRAVITY: Vector2<f32> = Vector2::new(0.0, -9.81);

#[derive(Resource)]
pub struct PhysicsWorld {
    pub gravity: Vector2<f32>,
    pub rigid_bodies: RigidBodySet,
    pub colliders: ColliderSet,
    pub simulation_steps: u64,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self {
            gravity: DEFAULT_GRAVITY,
            rigid_bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            simulation_steps: 0,
        }
    }
}

impl PhysicsWorld {
    pub fn new(gravity: Vector2<f32>) -> Self {
        Self {
            gravity,
            rigid_bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            simulation_steps: 0,
        }
    }

    /// Advance the physics world by one logical step.
    ///
    /// This currently only increments [`Self::simulation_steps`]. The full
    /// Rapier pipeline integration is planned for a future milestone.
    pub fn step(&mut self) {
        self.simulation_steps += 1;
    }
}

pub fn physics_sync_system(mut world: ResMut<PhysicsWorld>) {
    world.step();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_gravity_points_down() {
        let world = PhysicsWorld::default();
        assert_eq!(world.gravity, DEFAULT_GRAVITY);
        assert_eq!(world.simulation_steps, 0);
        assert!(world.rigid_bodies.is_empty());
        assert!(world.colliders.is_empty());
    }

    #[test]
    fn custom_gravity_is_stored() {
        let gravity = Vector2::new(0.0, -3.5);
        let world = PhysicsWorld::new(gravity);
        assert_eq!(world.gravity, gravity);
    }

    #[test]
    fn step_increments_counter() {
        let mut world = PhysicsWorld::default();
        world.step();
        world.step();
        world.step();
        assert_eq!(world.simulation_steps, 3);
    }

    #[test]
    fn physics_system_steps_world() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(physics_sync_system);
        schedule.run(&mut world);

        assert_eq!(
            world.get_resource::<PhysicsWorld>().unwrap().simulation_steps,
            1
        );
    }
}
