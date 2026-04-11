use bevy_ecs::prelude::*;
use rapier2d::na::Vector2;
use rapier2d::prelude::{ColliderSet, RigidBodySet};

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
            gravity: Vector2::new(0.0, -9.81),
            rigid_bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            simulation_steps: 0,
        }
    }
}

impl PhysicsWorld {
    pub fn step(&mut self) {
        self.simulation_steps += 1;
    }
}

pub fn physics_sync_system(mut world: ResMut<PhysicsWorld>) {
    world.step();
}
