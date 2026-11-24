//! Gamepad and navigation command primitives for BigPicture.
//!
//! [`gamepad::spawn`] roda o `gilrs` numa thread dedicada e publica [`NavCommand`] num canal;
//! o crate `app` converte os comandos em actions do GPUI. [`repeat`] e [`stick`] são a lógica
//! pura (repetição ao segurar e analógico como direcional), testável sem controle conectado.

pub mod gamepad;
pub mod repeat;
pub mod stick;

use bigpicture_core::Direction;

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

impl NavCommand {
    /// A direção espacial do comando, se for um direcional.
    pub fn direction(self) -> Option<Direction> {
        match self {
            NavCommand::Up => Some(Direction::Up),
            NavCommand::Down => Some(Direction::Down),
            NavCommand::Left => Some(Direction::Left),
            NavCommand::Right => Some(Direction::Right),
            _ => None,
        }
    }
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
        assert_eq!(NavCommand::Left.direction(), Some(Direction::Left));
        assert_eq!(NavCommand::Menu.direction(), None);
    }
}
