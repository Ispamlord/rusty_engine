fn update() {
    let count = object_count();
    if count < 3 {
        let id = spawn_object("SpawnedFromScript");
        set_transform(id, 120.0, 40.0, 0.0, 1.0, 1.0);
        set_sprite(id, "assets/basic_shapes/circle.ron", 48, 48);
        set_custom(id, "from_script", "true");
        emit_event("spawn", "SpawnedFromScript");
    }
}
