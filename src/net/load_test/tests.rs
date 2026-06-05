use super::{RoomCode, relay_join_url};

#[test]
fn load_join_url_includes_room_query_for_replay_routing() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();

    assert_eq!(
        relay_join_url("wss://typekart-relay.fly.dev", &room),
        "wss://typekart-relay.fly.dev/?typekart_room=rocket-salad-tiger"
    );
}
