//! Where the keyboard is (`docs/architecture/09-tui.md`, "Panels", "Tool
//! rounds"): the input, a live tool box, or a panel.
//!
//! `Ctrl+P` moves along a ring: the input, then each live tool box in the
//! order they are drawn, then each panel, then the input again. The caller
//! says what is there with [`Ring::update`] whenever it changes: the live
//! boxes come from the running round (step 23), the panels from the panel
//! mechanism (step 24). A target that goes away takes the focus back to the
//! input. Adding a kind of target is adding its ids to `update`; the ring
//! does not change.

/// Something that can have the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// The prompt's editor.
    Input,
    /// A live tool box, by the id its round gives it.
    ToolBox(u64),
    /// A panel, by the id the panel mechanism gives it.
    Panel(u64),
}

/// The focus ring.
#[derive(Debug, Clone, Default)]
pub struct Ring {
    boxes: Vec<u64>,
    panels: Vec<u64>,
    focused: Option<Target>,
}

impl Ring {
    /// A ring with only the input, which has the focus.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What has the focus.
    #[must_use]
    pub fn focused(&self) -> Target {
        self.focused.unwrap_or(Target::Input)
    }

    /// Whether the input has the focus.
    #[must_use]
    pub fn on_input(&self) -> bool {
        self.focused() == Target::Input
    }

    /// The live tool boxes and the panels there are now, in the order they
    /// are drawn. If what had the focus is gone, the input has it.
    pub fn update(&mut self, boxes: &[u64], panels: &[u64]) {
        self.boxes = boxes.to_vec();
        self.panels = panels.to_vec();
        if !self.targets().contains(&self.focused()) {
            self.focused = None;
        }
    }

    /// Every target, in ring order.
    #[must_use]
    pub fn targets(&self) -> Vec<Target> {
        std::iter::once(Target::Input)
            .chain(self.boxes.iter().map(|id| Target::ToolBox(*id)))
            .chain(self.panels.iter().map(|id| Target::Panel(*id)))
            .collect()
    }

    /// The focus moves to the next target, after the last back to the
    /// input. What has it now.
    pub fn advance(&mut self) -> Target {
        let targets = self.targets();
        let at = targets
            .iter()
            .position(|target| *target == self.focused())
            .unwrap_or(0);
        let next = targets[(at + 1) % targets.len()];
        self.focused = Some(next);
        next
    }

    /// The input takes the focus.
    pub fn to_input(&mut self) {
        self.focused = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_goes_through_boxes_then_panels_and_back() {
        let mut ring = Ring::new();
        assert_eq!(ring.advance(), Target::Input, "nothing else to go to");
        ring.update(&[7, 8], &[1]);
        assert_eq!(ring.advance(), Target::ToolBox(7));
        assert_eq!(ring.advance(), Target::ToolBox(8));
        assert_eq!(ring.advance(), Target::Panel(1));
        assert_eq!(ring.advance(), Target::Input);
    }

    #[test]
    fn a_target_that_goes_away_gives_the_focus_back_to_the_input() {
        let mut ring = Ring::new();
        ring.update(&[7], &[]);
        ring.advance();
        ring.update(&[], &[]);
        assert!(ring.on_input());
    }
}
