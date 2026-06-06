//! A scrollable portal whose viewport can be driven from app state.
//!
//! Xilem 0.4's built-in [`portal`](xilem::view::portal) view exposes no scroll-to API, so the
//! file tree can't programmatically scroll the diff through it. This thin custom view wraps the
//! same Masonry [`Portal`](xilem::masonry::widgets::Portal) widget and, whenever a one-shot
//! `generation` counter changes, jumps the viewport to a target vertical `fraction` (0..1) of the
//! content — letting "click a file leaf → scroll the diff toward it" actually work.
//!
//! The offset is approximate (computed from cumulative diff-row counts, which are uniform height),
//! not pixel-exact, because precise child layout rects aren't available at rebuild time. It lands
//! the clicked file near the top of the viewport, which is the intended behaviour.

use std::marker::PhantomData;

use xilem::core::{MessageContext, Mut, View, ViewMarker};
use xilem::masonry::core::Widget;
use xilem::masonry::kurbo::Point;
use xilem::masonry::widgets;
use xilem::{MessageResult, Pod, ViewCtx};

/// Create a [`ScrollPortal`] around an already type-erased child view.
pub fn scroll_portal<State, Action>(
    child: Box<xilem::AnyWidgetView<State, Action>>,
    generation: u64,
    fraction: Option<f64>,
) -> ScrollPortal<State, Action> {
    ScrollPortal {
        child,
        generation,
        fraction,
        phantom: PhantomData,
    }
}

#[must_use = "View values do nothing unless provided to Xilem."]
pub struct ScrollPortal<State, Action> {
    child: Box<xilem::AnyWidgetView<State, Action>>,
    generation: u64,
    fraction: Option<f64>,
    phantom: PhantomData<fn() -> (State, Action)>,
}

impl<State, Action> ViewMarker for ScrollPortal<State, Action> {}

impl<State, Action> View<State, Action, ViewCtx> for ScrollPortal<State, Action>
where
    State: 'static,
    Action: 'static,
{
    type Element = Pod<widgets::Portal<dyn Widget>>;
    type ViewState = <Box<xilem::AnyWidgetView<State, Action>> as View<State, Action, ViewCtx>>::ViewState;

    fn build(&self, ctx: &mut ViewCtx, app_state: &mut State) -> (Self::Element, Self::ViewState) {
        let (child, child_state) = self.child.build(ctx, app_state);
        let widget_pod = ctx.create_pod(widgets::Portal::new(child.new_widget));
        (widget_pod, child_state)
    }

    fn rebuild(
        &self,
        prev: &Self,
        view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) {
        {
            let child_element = widgets::Portal::child_mut(&mut element);
            self.child
                .rebuild(&prev.child, view_state, ctx, child_element, app_state);
        }
        // Apply a fresh scroll request only when the generation counter advanced (a new click),
        // so we don't fight the user's manual scrolling on unrelated rebuilds.
        if self.generation != prev.generation {
            if let Some(frac) = self.fraction {
                // content height = child size; pos is clamped inside `set_viewport_pos`.
                let content = widgets::Portal::child_mut(&mut element).ctx.size();
                let y = content.height * frac;
                widgets::Portal::set_viewport_pos(&mut element, Point::new(0.0, y));
            }
        }
    }

    fn teardown(
        &self,
        view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
    ) {
        let child_element = widgets::Portal::child_mut(&mut element);
        self.child.teardown(view_state, ctx, child_element);
    }

    fn message(
        &self,
        view_state: &mut Self::ViewState,
        message: &mut MessageContext,
        mut element: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) -> MessageResult<Action> {
        let child_element = widgets::Portal::child_mut(&mut element);
        self.child
            .message(view_state, message, child_element, app_state)
    }
}
