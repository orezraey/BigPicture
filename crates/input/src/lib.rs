//! Gamepad and navigation command primitives for BigPicture.

/// High-level navigation commands emitted by controller and keyboard inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NavCommand {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    Menu,
    Guide,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_command_variants_are_distinct() {
        let all = [
            NavCommand::Up,
            NavCommand::Down,
            NavCommand::Left,
            NavCommand::Right,
            NavCommand::Confirm,
            NavCommand::Back,
            NavCommand::Menu,
            NavCommand::Guide,
        ];
        assert_eq!(all.len(), 8);
        assert_ne!(NavCommand::Confirm, NavCommand::Back);
    }
}
