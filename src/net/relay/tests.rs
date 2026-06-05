use super::{RoomCode, generate_room_code};

#[test]
fn generated_room_codes_are_valid() {
    let code = generate_room_code();

    assert!(RoomCode::parse(code.as_str()).is_ok());
}
