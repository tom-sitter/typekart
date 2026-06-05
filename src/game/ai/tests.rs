use super::AiDifficulty;

#[test]
fn easy_and_hard_wpm_ranges_are_distinct() {
    let easy = AiDifficulty::Easy.wpm_range();
    let hard = AiDifficulty::Hard.wpm_range();

    assert!(easy.end() < hard.start());
}

#[test]
fn wpm_ranges_have_broad_spread() {
    let easy = AiDifficulty::Easy.wpm_range();
    let hard = AiDifficulty::Hard.wpm_range();

    assert_eq!((*easy.start(), *easy.end()), (20.0, 50.0));
    assert_eq!((*hard.start(), *hard.end()), (55.0, 105.0));
}
