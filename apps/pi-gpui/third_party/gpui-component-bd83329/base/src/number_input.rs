use std::rc::Rc;

use gpui::EventEmitter;

use crate::input::InputState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepAction {
    Decrement,
    Increment,
}

/// Strategy retained by the shared input engine for numeric stepping.
#[derive(Clone)]
pub enum NumberStep {
    Fixed(f64),
    ByValue(Rc<dyn Fn(f64, StepAction, &mut gpui::App) -> f64>),
}

impl NumberStep {
    pub fn by_value(f: impl Fn(f64, StepAction, &mut gpui::App) -> f64 + 'static) -> Self {
        Self::ByValue(Rc::new(f))
    }

    pub(crate) fn value(&self, current: f64, action: StepAction, cx: &mut gpui::App) -> f64 {
        match self {
            Self::Fixed(step) => *step,
            Self::ByValue(f) => f(current, action, cx),
        }
    }
}

impl From<f64> for NumberStep {
    fn from(step: f64) -> Self {
        Self::Fixed(step)
    }
}

#[derive(Clone)]
pub enum NumberInputEvent {
    Step(StepAction),
}

impl EventEmitter<NumberInputEvent> for InputState {}
