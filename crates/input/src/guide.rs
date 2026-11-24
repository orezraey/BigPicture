//! Botão Guide (Xbox) no Windows. O `XInputGetState` público, que o backend XInput do `gilrs`
//! usa, nunca informa o Guide; só a função não documentada `XInputGetStateEx` (ordinal 100 da
//! `xinput1_4.dll`) informa. Este módulo consulta essa função e gera `NavCommand::Guide` na
//! borda de descida. Como o XInput é por consulta e não depende de foco, funciona com um jogo na
//! frente do launcher.

/// Detecta a borda de descida do Guide em até quatro controles.
#[derive(Debug, Default)]
pub struct GuideEdges {
    held: [bool; 4],
}

impl GuideEdges {
    /// Atualiza o controle `slot` e diz se o Guide acabou de ser pressionado.
    pub fn update(&mut self, slot: usize, pressed: bool) -> bool {
        let Some(prev) = self.held.get_mut(slot) else { return false };
        let edge = pressed && !*prev;
        *prev = pressed;
        edge
    }
}

#[cfg(windows)]
pub struct GuidePoller {
    handle: Option<rusty_xinput::XInputHandle>,
    edges: GuideEdges,
}

#[cfg(windows)]
impl GuidePoller {
    pub fn new() -> Self {
        let handle = rusty_xinput::XInputHandle::load_default()
            .map_err(|err| tracing::warn!(error = ?err, "XInput indisponível; botão Guide desligado"))
            .ok();
        Self { handle, edges: GuideEdges::default() }
    }

    /// `true` se o Guide foi pressionado em algum controle desde a última consulta.
    pub fn poll(&mut self) -> bool {
        let Some(handle) = &self.handle else { return false };
        let mut pressed = false;
        for slot in 0..4u32 {
            let down = handle.get_state_ex(slot).is_ok_and(|state| state.guide_button());
            pressed |= self.edges.update(slot as usize, down);
        }
        pressed
    }
}

#[cfg(windows)]
impl Default for GuidePoller {
    fn default() -> Self {
        Self::new()
    }
}

/// Em outras plataformas o `gilrs` já entrega o Guide como `Button::Mode`.
#[cfg(not(windows))]
#[derive(Default)]
pub struct GuidePoller;

#[cfg(not(windows))]
impl GuidePoller {
    pub fn new() -> Self {
        Self
    }

    pub fn poll(&mut self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_once_per_press_per_slot() {
        let mut e = GuideEdges::default();
        assert!(e.update(0, true));
        assert!(!e.update(0, true), "segurar não repete");
        assert!(!e.update(0, false));
        assert!(e.update(0, true));
        assert!(e.update(1, true), "controles independentes");
        assert!(!e.update(9, true), "slot inexistente é ignorado");
    }
}
