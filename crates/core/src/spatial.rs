//! Navegação espacial pura: dado o item focado e uma direção, qual item recebe o foco.
//!
//! [`navigate_rects`] funciona para qualquer layout (menus, barras, grades irregulares).
//! [`GridMetrics::navigate`] é o caminho O(1) da grade de capas e dá o mesmo resultado que
//! [`navigate_rects`] sobre os retângulos da grade.

use crate::{GridMetrics, LogicalRect};

/// Direção de navegação (direcional do controle, analógico ou setas do teclado).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Escolhe o retângulo mais próximo de `rects[from]` na direção `dir`, ou `None` quando não
/// há nada naquela direção (o foco fica onde está; não dá a volta).
///
/// Candidatos são os retângulos cujo centro fica estritamente além da borda do atual naquela
/// direção. A pontuação é a distância no eixo da direção mais duas vezes o desalinhamento no
/// eixo perpendicular, de modo que o vizinho alinhado ganha de um mais perto na diagonal.
///
/// Primeiro valem só os candidatos que se sobrepõem ao atual no eixo perpendicular (o "feixe").
/// Esquerda e direita ficam nisso: não pulam de linha. Cima e baixo, sem ninguém no feixe,
/// aceitam qualquer candidato, para alcançar uma linha mais curta.
pub fn navigate_rects(rects: &[LogicalRect], from: usize, dir: Direction) -> Option<usize> {
    let cur = rects.get(from)?;
    let in_beam = |r: &LogicalRect| match dir {
        Direction::Left | Direction::Right => r.y < cur.bottom() && r.bottom() > cur.y,
        Direction::Up | Direction::Down => r.x < cur.right() && r.right() > cur.x,
    };
    best_in(rects, from, dir, &in_beam).or_else(|| match dir {
        Direction::Up | Direction::Down => best_in(rects, from, dir, &|_| true),
        Direction::Left | Direction::Right => None,
    })
}

fn best_in(
    rects: &[LogicalRect],
    from: usize,
    dir: Direction,
    keep: &dyn Fn(&LogicalRect) -> bool,
) -> Option<usize> {
    let cur = &rects[from];
    let (cx, cy) = (cur.center_x(), cur.center_y());
    let mut best: Option<(f32, usize)> = None;
    for (ix, r) in rects.iter().enumerate() {
        if ix == from || !keep(r) {
            continue;
        }
        let (rx, ry) = (r.center_x(), r.center_y());
        let (primary, ortho) = match dir {
            Direction::Up if ry < cur.y => (cur.y - r.bottom(), (rx - cx).abs()),
            Direction::Down if ry > cur.bottom() => (r.y - cur.bottom(), (rx - cx).abs()),
            Direction::Left if rx < cur.x => (cur.x - r.right(), (ry - cy).abs()),
            Direction::Right if rx > cur.right() => (r.x - cur.right(), (ry - cy).abs()),
            _ => continue,
        };
        let score = primary.max(0.0) + ortho * 2.0;
        // Empate: fica com o primeiro na ordem de leitura.
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, ix));
        }
    }
    best.map(|(_, ix)| ix)
}

impl GridMetrics {
    /// Navegação na grade de `count` itens em `columns` colunas, sem dar a volta nas bordas.
    ///
    /// Descer a partir de uma coluna que não existe na última linha (incompleta) vai para o
    /// último item, como fazem as grades de console; esquerda e direita não pulam de linha.
    pub fn navigate(&self, from: usize, columns: usize, count: usize, dir: Direction) -> Option<usize> {
        let cols = columns.max(1);
        if from >= count {
            return None;
        }
        let (row, col) = (from / cols, from % cols);
        let last_row = (count - 1) / cols;
        match dir {
            Direction::Left => (col > 0).then(|| from - 1),
            Direction::Right => (col + 1 < cols && from + 1 < count).then(|| from + 1),
            Direction::Up => (row > 0).then(|| from - cols),
            Direction::Down => (row < last_row).then(|| (from + cols).min(count - 1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_rects(m: &GridMetrics, count: usize, cols: usize) -> Vec<LogicalRect> {
        (0..count).map(|i| m.card_bounds(i, cols)).collect()
    }

    #[test]
    fn grid_moves_one_step_and_stops_at_edges() {
        let m = GridMetrics::default();
        // 3 colunas, 8 itens: linhas [0 1 2] [3 4 5] [6 7]
        assert_eq!(m.navigate(4, 3, 8, Direction::Left), Some(3));
        assert_eq!(m.navigate(4, 3, 8, Direction::Right), Some(5));
        assert_eq!(m.navigate(4, 3, 8, Direction::Up), Some(1));
        assert_eq!(m.navigate(4, 3, 8, Direction::Down), Some(7));
        assert_eq!(m.navigate(3, 3, 8, Direction::Left), None, "não volta para a linha de cima");
        assert_eq!(m.navigate(2, 3, 8, Direction::Right), None, "não desce para a próxima linha");
        assert_eq!(m.navigate(1, 3, 8, Direction::Up), None);
        assert_eq!(m.navigate(7, 3, 8, Direction::Down), None);
        assert_eq!(m.navigate(7, 3, 8, Direction::Right), None);
    }

    #[test]
    fn grid_down_into_short_last_row_lands_on_last_item() {
        let m = GridMetrics::default();
        assert_eq!(m.navigate(5, 3, 8, Direction::Down), Some(7));
        assert_eq!(m.navigate(999, 14, 1000, Direction::Down), None);
        assert_eq!(m.navigate(985, 14, 1000, Direction::Down), Some(999));
    }

    #[test]
    fn grid_rejects_out_of_range_focus() {
        let m = GridMetrics::default();
        assert_eq!(m.navigate(8, 3, 8, Direction::Up), None);
        assert_eq!(m.navigate(0, 3, 0, Direction::Down), None);
    }

    #[test]
    fn rect_navigation_matches_grid_fast_path() {
        let m = GridMetrics::default();
        for (count, cols) in [(8, 3), (30, 14), (29, 7), (1, 5)] {
            let rects = grid_rects(&m, count, cols);
            for from in 0..count {
                for dir in [Direction::Up, Direction::Down, Direction::Left, Direction::Right] {
                    let fast = m.navigate(from, cols, count, dir);
                    let general = navigate_rects(&rects, from, dir);
                    // A grade leva "baixo" na última linha incompleta para o último item; o
                    // caminho geral escolhe o vizinho mais próximo, que é esse mesmo item.
                    assert_eq!(fast, general, "count={count} cols={cols} from={from} dir={dir:?}");
                }
            }
        }
    }

    #[test]
    fn rect_navigation_prefers_aligned_neighbor() {
        // A: origem. B: à direita e um pouco abaixo. C: abaixo de A, mais longe que B em y.
        let rects = [
            LogicalRect::new(0.0, 0.0, 100.0, 40.0),
            LogicalRect::new(150.0, 30.0, 100.0, 40.0),
            LogicalRect::new(0.0, 120.0, 100.0, 40.0),
        ];
        assert_eq!(navigate_rects(&rects, 0, Direction::Down), Some(2), "C está no feixe");
        assert_eq!(navigate_rects(&rects, 0, Direction::Right), Some(1));
        assert_eq!(navigate_rects(&rects, 2, Direction::Right), None, "B fora do feixe");
        assert_eq!(navigate_rects(&rects, 0, Direction::Up), None);
        assert_eq!(navigate_rects(&rects, 9, Direction::Up), None);
    }
}
