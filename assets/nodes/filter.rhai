fn execute() {
    let id = spawn_object("FilterLogic");
    set_transform(id, 224.0, 96.0, 0.0, 1.0, 1.0);
    set_sprite(id, "assets/basic_shapes/triangle.ron", 40, 40);
    set_custom(id, "node", "filter");
    emit_event("custom_node", "filter execute");
}
