fn execute() {
    let id = spawn_object("MathLogic");
    set_transform(id, 160.0, 96.0, 0.0, 1.0, 1.0);
    set_sprite(id, "assets/basic_shapes/square.ron", 40, 40);
    set_custom(id, "node", "math");
    emit_event("custom_node", "math execute");
}
