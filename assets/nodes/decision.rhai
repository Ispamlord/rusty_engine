fn execute() {
    let id = spawn_object("DecisionLogic");
    set_transform(id, 288.0, 96.0, 0.0, 1.0, 1.0);
    set_sprite(id, "assets/basic_shapes/circle.ron", 40, 40);
    set_custom(id, "node", "decision");
    emit_event("custom_node", "decision execute");
}
