fn execute() {
    let id = spawn_object("CustomLogic");
    set_transform(id, 96.0, 96.0, 0.0, 1.0, 1.0);
    set_sprite(id, "assets/basic_shapes/circle.ron", 40, 40);
    set_custom(id, "node", "minimal");
    emit_event("custom_node", "minimal execute");
}
