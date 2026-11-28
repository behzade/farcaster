use gpui::{Bounds, Pixels, Styled as _, Window, canvas};
use objc2::rc::Retained;
use objc2_core_graphics::CGColor;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CATransaction};

use super::RaisedEdge;
use crate::app::views::native_overlay::native::NativeHost;

#[derive(Default)]
struct Shadow {
    parent: Option<Retained<CALayer>>,
    layer: Retained<CALayer>,
    samples: Vec<Retained<CALayer>>,
}

impl Shadow {
    fn draw(&mut self, bounds: Bounds<Pixels>, edge: RaisedEdge, opacity: f32, window: &Window) {
        if self.parent.is_none() {
            self.parent = NativeHost::decoration_layer(window);
        }
        let Some(parent) = self.parent.as_ref() else {
            return;
        };
        let x = f64::from(f32::from(bounds.origin.x));
        let y = f64::from(f32::from(bounds.origin.y));
        let width = f64::from(f32::from(bounds.size.width));
        let height = f64::from(f32::from(bounds.size.height));
        let y = if parent.isGeometryFlipped() {
            y
        } else {
            parent.bounds().size.height - y - height
        };
        let depth = match edge {
            RaisedEdge::Top => height,
            _ => width,
        };
        let count = (depth * f64::from(window.scale_factor())).ceil().max(1.0) as usize;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.layer
            .setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
        self.layer.setGeometryFlipped(true);
        self.layer.setZPosition(1.0);
        while self.samples.len() < count {
            let sample = CALayer::layer();
            self.layer.addSublayer(&sample);
            self.samples.push(sample);
        }
        for sample in self.samples.drain(count..) {
            sample.removeFromSuperlayer();
        }
        let step = depth / count as f64;
        for (index, sample) in self.samples.iter().enumerate() {
            let offset = index as f64 * step;
            let (x, y, width, height) = match edge {
                RaisedEdge::Top => (0.0, offset, width, step),
                RaisedEdge::Left => (offset, 0.0, step, height),
                RaisedEdge::Right => (width - offset - step, 0.0, step, height),
            };
            sample.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
            sample.setBackgroundColor(Some(&CGColor::new_generic_rgb(
                0.0,
                0.0,
                0.0,
                f64::from(opacity) * (1.0 - (index as f64 + 0.5) / count as f64),
            )));
        }
        parent.addSublayer(&self.layer);
        CATransaction::commit();
    }
}

impl Drop for Shadow {
    fn drop(&mut self) {
        self.layer.removeFromSuperlayer();
    }
}

pub(super) fn shadow(edge: RaisedEdge, opacity: f32) -> impl gpui::IntoElement {
    canvas(
        move |bounds, window, cx| {
            let key = match edge {
                RaisedEdge::Top => "native-top-shadow",
                RaisedEdge::Left => "native-left-shadow",
                RaisedEdge::Right => "native-right-shadow",
            };
            let shadow = window.use_keyed_state(key, cx, |_, _| Shadow::default());
            shadow.update(cx, |shadow, _| shadow.draw(bounds, edge, opacity, window));
        },
        |_, _, _, _| {},
    )
    .size_full()
}
