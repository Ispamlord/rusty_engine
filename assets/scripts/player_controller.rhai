fn update() {
    let hero = find_object("Hero");
    if hero < 0 {
        return;
    }

    let speed = get_custom_f32(hero, "speed", 2.5);
    let dx = 0.0;
    let dy = 0.0;

    if key_down("left") {
        dx = dx - speed;
    }
    if key_down("right") {
        dx = dx + speed;
    }
    if key_down("up") {
        dy = dy - speed;
    }
    if key_down("down") {
        dy = dy + speed;
    }

    if dx == 0.0 && dy == 0.0 {
        return;
    }

    let moved = move_with_collision(hero, dx, dy);
    if !moved {
        emit_event("collision", "Hero hit obstacle");
    }
}
