//! Virtual List Primitive
//!
//! A high-performance virtualized list component that only renders visible items.
//! Inspired by Android's RecyclerView, this component can handle large lists
//! while maintaining smooth scrolling.
//!
//! # Design
//!
//! This is a primitive component that implements iced's `Widget` trait.
//! It uses generic Message and Theme types and does not depend on
//! application-specific types.
//!
//! # Key Features
//!
//! - Tree Diffing: Preserves widget state (focus, animations, etc.) when scrolling
//! - Scrollbar: Visual indicator of scroll position
//! - Buffer items: Renders extra items above/below viewport for smooth scrolling
//! - Element Caching: Each item_builder is called only once per frame
//! - Optimized: Minimizes item_builder calls per frame

use iced::advanced::Shell;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::mouse::{self, Cursor};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

use crate::responsive::UiTokens;

/// Buffer items to render above and below the visible area
const BUFFER_ITEMS: usize = 8;
/// Number of rows used before the first real layout reports a viewport size.
/// This keeps initial image demand bounded while the widget is bootstrapping.
const INITIAL_VIEWPORT_ITEMS: usize = 10;

type VisibleRangeCallback<'a, Message> = Box<dyn Fn((usize, usize)) -> Message + 'a>;

/// Events that must reach every mounted row because child widgets may own
/// lifecycle, focus, or clipboard state that is independent of pointer hit
/// testing. In particular, stateful image widgets synchronize new handles on
/// `RedrawRequested`.
fn broadcasts_to_visible_children(event: &Event) -> bool {
    matches!(
        event,
        Event::Keyboard(_) | Event::Window(_) | Event::InputMethod(_) | Event::Clipboard(_)
    )
}

/// State for the virtual list
#[derive(Debug, Clone)]
pub struct VirtualListState {
    /// Current scroll offset in pixels
    pub scroll_offset: f32,
    /// Viewport height in pixels
    pub viewport_height: f32,
    /// Total item count
    pub item_count: usize,
    /// Item height
    pub item_height: f32,
}

impl Default for VirtualListState {
    fn default() -> Self {
        Self {
            scroll_offset: 0.0,
            viewport_height: 0.0,
            item_count: 0,
            item_height: 0.0,
        }
    }
}

impl VirtualListState {
    /// Create a new state with the given item count and default height
    pub fn new(item_count: usize, item_height: f32) -> Self {
        Self {
            scroll_offset: 0.0,
            viewport_height: 0.0,
            item_count,
            item_height,
        }
    }

    /// Get total content height
    pub fn total_height(&self) -> f32 {
        self.item_count as f32 * self.item_height
    }

    /// Calculate visible range with buffer
    pub fn visible_range(&self) -> (usize, usize) {
        if self.item_count == 0 {
            return (0, 0);
        }

        if self.viewport_height <= 0.0 || self.item_height <= 0.0 {
            return (
                0,
                (INITIAL_VIEWPORT_ITEMS + BUFFER_ITEMS).min(self.item_count),
            );
        }

        let first_visible = (self.scroll_offset / self.item_height).floor() as usize;
        let visible_count = (self.viewport_height / self.item_height).ceil() as usize + 1;

        let start = first_visible.saturating_sub(BUFFER_ITEMS);
        let end = (first_visible + visible_count + BUFFER_ITEMS).min(self.item_count);

        (start, end)
    }

    /// Update state
    pub fn update(&mut self, item_count: usize, item_height: f32, viewport_height: f32) {
        self.item_count = item_count;
        self.item_height = item_height;
        self.viewport_height = viewport_height;

        // Clamp scroll offset
        let max_scroll = self.max_scroll();
        self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll);
    }

    /// Get maximum scroll offset
    pub fn max_scroll(&self) -> f32 {
        (self.total_height() - self.viewport_height).max(0.0)
    }

    /// Applies a relative pixel offset immediately and returns the distance
    /// that was actually consumed after clamping.
    pub fn scroll_by_immediate(&mut self, delta: f32) -> f32 {
        let previous = self.scroll_offset;
        self.jump_to(previous + delta);
        self.scroll_offset - previous
    }

    /// Jumps to an absolute pixel offset, clamped to the current content.
    pub fn jump_to(&mut self, offset: f32) {
        self.scroll_offset = offset.clamp(0.0, self.max_scroll());
    }
}

/// A virtual list widget that only renders visible items
pub struct VirtualList<'a, Message, Theme, Renderer, Key = usize>
where
    Renderer: renderer::Renderer,
    Key: Eq + Hash + Clone + 'static,
{
    /// Total number of items
    item_count: usize,
    /// Item height
    item_height: f32,
    /// Function to build an item element by index
    item_builder: Box<dyn Fn(usize) -> Element<'a, Message, Theme, Renderer> + 'a>,
    /// Function to return a stable identity key for an item index
    item_key: Box<dyn Fn(usize) -> Key + 'a>,
    /// Shared state
    state: Rc<RefCell<VirtualListState>>,
    /// Width of the list
    width: Length,
    /// Height of the list
    height: Length,
    /// Whether to show scrollbar
    show_scrollbar: bool,
    /// Root-rem tokens for renderer-owned scrollbar and line-wheel metrics.
    tokens: UiTokens,
    /// Message to send when mouse moves over empty area (not over any item)
    on_empty_area: Option<Message>,
    /// Function to create hover message for an item index
    on_item_hover: Option<Box<dyn Fn(usize) -> Message + 'a>>,
    /// Function to create a message when the visible range changes.
    on_visible_range: Option<VisibleRangeCallback<'a, Message>>,
    /// Optional callback used instead of directly applying line-wheel input.
    on_smooth_scroll: Option<Box<dyn Fn(f32) -> Message + 'a>>,
    /// Message emitted before immediate pixel or scrollbar movement.
    on_smooth_scroll_cancel: Option<Message>,
    /// Optional owner token used to force a fresh range notification when a
    /// page reuses the same widget tree with different image ownership.
    visible_range_token: Option<u64>,
}

impl<'a, Message, Theme, Renderer> VirtualList<'a, Message, Theme, Renderer, usize>
where
    Renderer: renderer::Renderer,
{
    /// Create a new virtual list
    pub fn new<F>(item_count: usize, item_height: f32, tokens: UiTokens, item_builder: F) -> Self
    where
        F: Fn(usize) -> Element<'a, Message, Theme, Renderer> + 'a,
    {
        let state = Rc::new(RefCell::new(VirtualListState::new(item_count, item_height)));
        Self {
            item_count,
            item_height,
            item_builder: Box::new(item_builder),
            item_key: Box::new(|index| index),
            state,
            width: Length::Fill,
            height: Length::Fill,
            show_scrollbar: true,
            tokens,
            on_empty_area: None,
            on_item_hover: None,
            on_visible_range: None,
            on_smooth_scroll: None,
            on_smooth_scroll_cancel: None,
            visible_range_token: None,
        }
    }
}

impl<'a, Message, Theme, Renderer, Key> VirtualList<'a, Message, Theme, Renderer, Key>
where
    Renderer: renderer::Renderer,
    Key: Eq + Hash + Clone + 'static,
{
    /// Set the width of the list
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Set the height of the list
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Set external state (for persistence across frames)
    pub fn state(mut self, state: Rc<RefCell<VirtualListState>>) -> Self {
        self.state = state;
        self
    }

    /// Set spacing between items (not used in fixed height mode)
    pub fn spacing(self, _spacing: f32) -> Self {
        self
    }

    /// Show or hide the scrollbar
    pub fn scrollbar(mut self, show: bool) -> Self {
        self.show_scrollbar = show;
        self
    }

    /// Set a message to send when mouse moves over empty area (not over any item)
    /// This is useful for clearing hover states when mouse leaves all items
    pub fn on_empty_area(mut self, message: Message) -> Self {
        self.on_empty_area = Some(message);
        self
    }

    /// Set a callback to create hover message for each item
    /// This is called on every mouse move to update hover state reliably
    /// even when mouse moves fast between items
    pub fn on_item_hover<F>(mut self, f: F) -> Self
    where
        F: Fn(usize) -> Message + 'a,
    {
        self.on_item_hover = Some(Box::new(f));
        self
    }

    /// Set a callback for the current visible range, including the built-in
    /// overscan buffer. The callback is emitted only when the range changes.
    pub fn on_visible_range<F>(mut self, f: F) -> Self
    where
        F: Fn((usize, usize)) -> Message + 'a,
    {
        self.on_visible_range = Some(Box::new(f));
        self
    }

    /// Routes vertical line-wheel input through an application callback.
    pub fn on_smooth_scroll<F>(mut self, f: F) -> Self
    where
        F: Fn(f32) -> Message + 'a,
    {
        self.on_smooth_scroll = Some(Box::new(f));
        self
    }

    /// Sets the message emitted when immediate input takes over scrolling.
    pub fn on_smooth_scroll_cancel(mut self, message: Message) -> Self {
        self.on_smooth_scroll_cancel = Some(message);
        self
    }

    /// Set an owner token for visible-range notifications. A changed token
    /// emits the current range again even when the numeric range is unchanged.
    pub fn visible_range_token(mut self, token: u64) -> Self {
        self.visible_range_token = Some(token);
        self
    }

    /// Set a stable identity key for each item.
    ///
    /// Use a domain identifier (for example, a track id plus original index) when
    /// list order can change because of filtering, sorting, or pagination.
    pub fn keyed_by<NewKey, F>(self, f: F) -> VirtualList<'a, Message, Theme, Renderer, NewKey>
    where
        NewKey: Eq + Hash + Clone + 'static,
        F: Fn(usize) -> NewKey + 'a,
    {
        VirtualList {
            item_count: self.item_count,
            item_height: self.item_height,
            item_builder: self.item_builder,
            item_key: Box::new(f),
            state: self.state,
            width: self.width,
            height: self.height,
            show_scrollbar: self.show_scrollbar,
            tokens: self.tokens,
            on_empty_area: self.on_empty_area,
            on_item_hover: self.on_item_hover,
            on_visible_range: self.on_visible_range,
            on_smooth_scroll: self.on_smooth_scroll,
            on_smooth_scroll_cancel: self.on_smooth_scroll_cancel,
            visible_range_token: self.visible_range_token,
        }
    }

    fn publish_visible_range(
        &self,
        internal_state: &mut VirtualListInternalState<Key>,
        shell: &mut Shell<'_, Message>,
    ) {
        let range = self.state.borrow().visible_range();
        if internal_state.last_notified_token != self.visible_range_token {
            internal_state.last_notified_visible_range = None;
            internal_state.last_notified_token = self.visible_range_token;
        }
        if internal_state.last_notified_visible_range == Some(range) {
            return;
        }

        internal_state.last_notified_visible_range = Some(range);
        if let Some(on_visible_range) = &self.on_visible_range {
            shell.publish(on_visible_range(range));
        }
    }
}

/// Internal state for widget tree
struct VirtualListInternalState<Key> {
    /// Trees keyed by item identity (persistent across scrolls and filtering)
    item_trees: HashMap<Key, Tree>,
    /// Cached visible range from layout phase
    cached_visible_range: (usize, usize),
    /// Whether the scrollbar is being dragged
    scrollbar_dragging: bool,
    /// The Y position where drag started (relative to scrollbar top)
    drag_start_offset: f32,
    /// Whether mouse is hovering over scrollbar
    scrollbar_hovered: bool,
    /// Last hovered item key for deduplication
    last_hovered_item: Option<Key>,
    /// Last visible range reported to the application.
    last_notified_visible_range: Option<(usize, usize)>,
    /// Owner token associated with the last visible-range notification.
    last_notified_token: Option<u64>,
    /// Stable keys in the last diffed visible/overscan range.
    last_visible_keys: Vec<Key>,
}

impl<Key> Default for VirtualListInternalState<Key> {
    fn default() -> Self {
        Self {
            item_trees: HashMap::new(),
            cached_visible_range: (0, 0),
            scrollbar_dragging: false,
            drag_start_offset: 0.0,
            scrollbar_hovered: false,
            last_hovered_item: None,
            last_notified_visible_range: None,
            last_notified_token: None,
            last_visible_keys: Vec::new(),
        }
    }
}

impl<'a, Message, Theme, Renderer, Key> Widget<Message, Theme, Renderer>
    for VirtualList<'a, Message, Theme, Renderer, Key>
where
    Message: Clone + 'a,
    Renderer: renderer::Renderer,
    Key: Eq + Hash + Clone + 'static,
{
    fn diff(&mut self, tree: &mut Tree) {
        let (start, end) = {
            let mut state = self.state.borrow_mut();
            state.item_count = self.item_count;
            state.item_height = self.item_height;
            state.visible_range()
        };

        let internal_state = tree.state.downcast_mut::<VirtualListInternalState<Key>>();

        internal_state.cached_visible_range = (start, end);
        let visible_keys: Vec<Key> = (start..end).map(|idx| (self.item_key)(idx)).collect();
        if internal_state.last_visible_keys != visible_keys {
            internal_state.last_visible_keys = visible_keys;
            internal_state.last_notified_visible_range = None;
        }

        // Prune trees for items far outside the visible range (keep a generous buffer)
        let prune_start = start.saturating_sub(BUFFER_ITEMS * 2);
        let prune_end = (end + BUFFER_ITEMS * 2).min(self.item_count);
        let retained_keys: HashSet<Key> = (prune_start..prune_end)
            .map(|idx| (self.item_key)(idx))
            .collect();
        internal_state
            .item_trees
            .retain(|key, _| retained_keys.contains(key));

        // Ensure trees exist for all visible items and diff them
        for item_idx in start..end {
            let mut element = (self.item_builder)(item_idx);
            let item_key = (self.item_key)(item_idx);
            let tree = internal_state
                .item_trees
                .entry(item_key)
                .or_insert_with(Tree::empty);
            tree.diff(&mut element);
        }
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<VirtualListInternalState<Key>>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(VirtualListInternalState::<Key>::default())
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width).height(self.height);
        let size = limits.resolve(self.width, self.height, Size::ZERO);

        // Update state
        {
            let mut state = self.state.borrow_mut();
            state.update(self.item_count, self.item_height, size.height);
        }

        let state = self.state.borrow();
        let (start, end) = state.visible_range();
        let visible_count = end - start;
        let scroll_offset = state.scroll_offset;
        drop(state);

        // Get internal state
        let internal_state = tree.state.downcast_mut::<VirtualListInternalState<Key>>();

        // Cache the visible range
        internal_state.cached_visible_range = (start, end);

        // Build layout for visible items
        let mut children = Vec::with_capacity(visible_count);
        let item_limits = layout::Limits::new(Size::ZERO, Size::new(size.width, self.item_height));

        for item_idx in start..end {
            let mut element = (self.item_builder)(item_idx);
            let item_key = (self.item_key)(item_idx);

            // Get tree — normally diffed in diff(), but range may have changed
            // if viewport_height was updated between diff() and layout().
            let tree = internal_state
                .item_trees
                .entry(item_key)
                .or_insert_with(|| {
                    let mut t = Tree::empty();
                    t.diff(&mut element);
                    t
                });
            tree.diff(&mut element);

            let node = element.as_widget_mut().layout(tree, renderer, &item_limits);

            let y_position = item_idx as f32 * self.item_height - scroll_offset;
            let positioned = node.move_to(Point::new(0.0, y_position));
            children.push(positioned);
        }

        layout::Node::with_children(size, children)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let internal_state = tree.state.downcast_ref::<VirtualListInternalState<Key>>();
        let (start, end) = internal_state.cached_visible_range;

        let Some(visible_bounds) = bounds.intersection(viewport) else {
            return;
        };

        // Early return if no items to draw
        if start >= end {
            return;
        }

        // Find the subset of items actually on screen (within viewport bounds)
        let scroll_offset = self.state.borrow().scroll_offset;
        let visible_top = visible_bounds.y - bounds.y;
        let visible_bottom = visible_top + visible_bounds.height;
        let mut on_screen_start = end;
        let mut on_screen_end = start;
        for item_idx in start..end {
            let y = item_idx as f32 * self.item_height - scroll_offset;
            let y_end = y + self.item_height;
            if y_end > visible_top && y < visible_bottom {
                if item_idx < on_screen_start {
                    on_screen_start = item_idx;
                }
                on_screen_end = item_idx + 1;
            }
        }

        if on_screen_start >= on_screen_end {
            return;
        }

        // Draw list items inside both the list bounds and the parent viewport.
        renderer.with_layer(visible_bounds, |renderer| {
            let mut children = layout.children();
            // Skip to the first on-screen item
            let skip = on_screen_start.saturating_sub(start);
            if skip > 0 && children.nth(skip - 1).is_none() {
                return;
            }

            for item_idx in on_screen_start..on_screen_end {
                let item_key = (self.item_key)(item_idx);
                let child_layout = match children.next() {
                    Some(l) => l,
                    None => break,
                };

                if let Some(child_tree) = internal_state.item_trees.get(&item_key) {
                    let element = (self.item_builder)(item_idx);
                    element.as_widget().draw(
                        child_tree,
                        renderer,
                        theme,
                        style,
                        child_layout,
                        cursor,
                        &visible_bounds,
                    );
                }
            }
        });

        // Draw scrollbar overlay
        if self.show_scrollbar {
            let state = self.state.borrow();
            if state.total_height() > bounds.height {
                renderer.with_layer(visible_bounds, |renderer| {
                    self.draw_scrollbar(
                        renderer,
                        bounds,
                        &state,
                        internal_state.scrollbar_hovered,
                        internal_state.scrollbar_dragging,
                    );
                });
            }
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let internal_state = tree.state.downcast_mut::<VirtualListInternalState<Key>>();

        self.publish_visible_range(internal_state, shell);

        let scrollbar_bounds = {
            let state = self.state.borrow();
            self.calculate_scrollbar_bounds(bounds, &state)
        };

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position()
                    && let Some(sb_bounds) = scrollbar_bounds
                    && sb_bounds.contains(position)
                {
                    if let Some(message) = &self.on_smooth_scroll_cancel {
                        shell.publish(message.clone());
                    }
                    internal_state.scrollbar_dragging = true;
                    internal_state.drag_start_offset = position.y - sb_bounds.y;
                    shell.capture_event();
                    return;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if internal_state.scrollbar_dragging =>
            {
                internal_state.scrollbar_dragging = false;
                shell.capture_event();
                return;
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                // A `CursorMoved` event can still reach a lower stack layer with a
                // `Cursor::Levitating`/`Unavailable` cursor when an upper layer owns the
                // pointer. Do not use the raw event position in that case: doing so would let
                // this custom widget update hover state even though Iced has deliberately hidden
                // the cursor from the layer below.
                let Some(position) = cursor.position() else {
                    if internal_state.scrollbar_hovered {
                        internal_state.scrollbar_hovered = false;
                        shell.request_redraw();
                    }
                    if internal_state.last_hovered_item.is_some() {
                        internal_state.last_hovered_item = None;
                        if let Some(msg) = &self.on_empty_area {
                            shell.publish(msg.clone());
                        }
                    }
                    return;
                };

                if let Some(sb_bounds) = scrollbar_bounds {
                    let was_hovered = internal_state.scrollbar_hovered;
                    internal_state.scrollbar_hovered = sb_bounds.contains(position);
                    if was_hovered != internal_state.scrollbar_hovered {
                        shell.request_redraw();
                    }
                }

                if internal_state.scrollbar_dragging {
                    let state_ref = self.state.borrow();
                    let total_height = state_ref.total_height();
                    let max_scroll = state_ref.max_scroll();
                    drop(state_ref);

                    if max_scroll > 0.0 && total_height > 0.0 {
                        let view_ratio = bounds.height / total_height;
                        let scrollbar_height =
                            (bounds.height * view_ratio).max(self.tokens.size(30.0));
                        let available_track = bounds.height - scrollbar_height;

                        if available_track > 0.0 {
                            let scrollbar_top =
                                position.y - bounds.y - internal_state.drag_start_offset;
                            let scroll_ratio = (scrollbar_top / available_track).clamp(0.0, 1.0);
                            let new_offset = scroll_ratio * max_scroll;

                            let mut state = self.state.borrow_mut();
                            if (new_offset - state.scroll_offset).abs() > 0.01 {
                                state.scroll_offset = new_offset;
                                shell.invalidate_layout();
                                drop(state);
                                self.publish_visible_range(internal_state, shell);
                            }
                        }
                    }
                    shell.capture_event();
                    return;
                }

                // Handle hover state directly on CursorMoved for reliable tracking
                if bounds.contains(position) {
                    if let Some(on_hover) = &self.on_item_hover {
                        let state = self.state.borrow();
                        let scroll_offset = state.scroll_offset;
                        let item_count = state.item_count;
                        drop(state);

                        let relative_y = position.y - bounds.y + scroll_offset;
                        let target_item_idx = (relative_y / self.item_height).floor() as usize;

                        if target_item_idx < item_count {
                            let target_item_key = (self.item_key)(target_item_idx);
                            if internal_state.last_hovered_item.as_ref() != Some(&target_item_key) {
                                internal_state.last_hovered_item = Some(target_item_key);
                                shell.publish((on_hover)(target_item_idx));
                            }
                        } else if internal_state.last_hovered_item.is_some() {
                            internal_state.last_hovered_item = None;
                            if let Some(msg) = &self.on_empty_area {
                                shell.publish(msg.clone());
                            }
                        }
                    }
                } else if internal_state.last_hovered_item.is_some() {
                    internal_state.last_hovered_item = None;
                    if let Some(msg) = &self.on_empty_area {
                        shell.publish(msg.clone());
                    }
                }
            }
            _ => {}
        }

        if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event
            && let Some(position) = cursor.position()
            && bounds.contains(position)
        {
            match delta {
                mouse::ScrollDelta::Lines { y, .. } if self.on_smooth_scroll.is_some() => {
                    let delta = -*y * self.tokens.size(50.0);
                    if delta.abs() > f32::EPSILON
                        && let Some(on_smooth_scroll) = &self.on_smooth_scroll
                    {
                        shell.publish(on_smooth_scroll(delta));
                    }
                }
                mouse::ScrollDelta::Lines { y, .. } => {
                    let mut state = self.state.borrow_mut();
                    if state
                        .scroll_by_immediate(-*y * self.tokens.size(50.0))
                        .abs()
                        > 0.01
                    {
                        shell.invalidate_layout();
                        drop(state);
                        self.publish_visible_range(internal_state, shell);
                    }
                }
                mouse::ScrollDelta::Pixels { y, .. } => {
                    if let Some(message) = &self.on_smooth_scroll_cancel {
                        shell.publish(message.clone());
                    }
                    let mut state = self.state.borrow_mut();
                    if state.scroll_by_immediate(-*y).abs() > 0.01 {
                        shell.invalidate_layout();
                        drop(state);
                        self.publish_visible_range(internal_state, shell);
                    }
                }
            }
            shell.capture_event();
        }

        if internal_state.scrollbar_dragging {
            return;
        }

        let cursor_pos = cursor.position();
        let cursor_in_bounds = cursor_pos.map(|pos| bounds.contains(pos)).unwrap_or(false);

        match event {
            Event::Mouse(_) => {
                if !cursor_in_bounds {
                    return;
                }

                if let Some(pos) = cursor_pos {
                    let state = self.state.borrow();
                    let scroll_offset = state.scroll_offset;
                    let item_count = state.item_count;
                    drop(state);

                    let relative_y = pos.y - bounds.y + scroll_offset;
                    let target_item_idx = (relative_y / self.item_height).floor() as usize;

                    // Check if mouse is over empty area (beyond the last item)
                    if target_item_idx >= item_count {
                        if internal_state.last_hovered_item.is_some() {
                            internal_state.last_hovered_item = None;
                            if let Some(msg) = &self.on_empty_area {
                                shell.publish(msg.clone());
                            }
                        }
                        return;
                    }

                    let (cached_start, _) = internal_state.cached_visible_range;
                    if target_item_idx >= cached_start {
                        let slot_idx = target_item_idx - cached_start;
                        let item_key = (self.item_key)(target_item_idx);
                        if let Some(child_tree) = internal_state.item_trees.get_mut(&item_key) {
                            let mut children = layout.children();
                            if let Some(child_layout) = children.nth(slot_idx) {
                                let mut element = (self.item_builder)(target_item_idx);
                                element.as_widget_mut().update(
                                    child_tree,
                                    event,
                                    child_layout,
                                    cursor,
                                    renderer,
                                    shell,
                                    viewport,
                                );
                            }
                        }
                    }
                }
            }

            Event::Touch(_) => {
                if !cursor_in_bounds {
                    return;
                }

                if let Some(pos) = cursor_pos {
                    let state = self.state.borrow();
                    let scroll_offset = state.scroll_offset;
                    drop(state);

                    let relative_y = pos.y - bounds.y + scroll_offset;
                    let target_item_idx = (relative_y / self.item_height).floor() as usize;

                    let (cached_start, _) = internal_state.cached_visible_range;
                    if target_item_idx >= cached_start {
                        let slot_idx = target_item_idx - cached_start;
                        let item_key = (self.item_key)(target_item_idx);
                        if let Some(child_tree) = internal_state.item_trees.get_mut(&item_key) {
                            let mut children = layout.children();
                            if let Some(child_layout) = children.nth(slot_idx) {
                                let mut element = (self.item_builder)(target_item_idx);
                                element.as_widget_mut().update(
                                    child_tree,
                                    event,
                                    child_layout,
                                    cursor,
                                    renderer,
                                    shell,
                                    viewport,
                                );
                            }
                        }
                    }
                }
            }

            _ if broadcasts_to_visible_children(event) => {
                let (start, end) = internal_state.cached_visible_range;
                let mut children = layout.children();
                for item_idx in start..end {
                    let item_key = (self.item_key)(item_idx);
                    if let Some(child_layout) = children.next()
                        && let Some(child_tree) = internal_state.item_trees.get_mut(&item_key)
                    {
                        let mut element = (self.item_builder)(item_idx);
                        element.as_widget_mut().update(
                            child_tree,
                            event,
                            child_layout,
                            cursor,
                            renderer,
                            shell,
                            viewport,
                        );
                    }
                }
            }

            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let bounds = layout.bounds();
        let internal_state = tree.state.downcast_ref::<VirtualListInternalState<Key>>();

        if internal_state.scrollbar_dragging {
            return mouse::Interaction::Grabbing;
        }

        let cursor_pos = match cursor.position() {
            Some(pos) => pos,
            None => return mouse::Interaction::default(),
        };

        if !bounds.contains(cursor_pos) {
            return mouse::Interaction::default();
        }

        let state = self.state.borrow();
        if let Some(sb_bounds) = self.calculate_scrollbar_bounds(bounds, &state)
            && sb_bounds.contains(cursor_pos)
        {
            return mouse::Interaction::Grab;
        }
        let scroll_offset = state.scroll_offset;
        drop(state);

        let relative_y = cursor_pos.y - bounds.y + scroll_offset;
        let target_item_idx = (relative_y / self.item_height).floor() as usize;

        let (cached_start, _) = internal_state.cached_visible_range;
        if target_item_idx >= cached_start {
            let slot_idx = target_item_idx - cached_start;
            let item_key = (self.item_key)(target_item_idx);
            if let Some(child_tree) = internal_state.item_trees.get(&item_key) {
                let mut children = layout.children();
                if let Some(child_layout) = children.nth(slot_idx) {
                    let element = (self.item_builder)(target_item_idx);
                    return element.as_widget().mouse_interaction(
                        child_tree,
                        child_layout,
                        cursor,
                        viewport,
                        renderer,
                    );
                }
            }
        }

        mouse::Interaction::default()
    }
}

impl<'a, Message, Theme, Renderer, Key> VirtualList<'a, Message, Theme, Renderer, Key>
where
    Renderer: renderer::Renderer,
    Key: Eq + Hash + Clone + 'static,
{
    fn calculate_scrollbar_bounds(
        &self,
        bounds: Rectangle,
        state: &VirtualListState,
    ) -> Option<Rectangle> {
        if !self.show_scrollbar {
            return None;
        }

        let total_height = state.total_height();
        let max_scroll = state.max_scroll();

        if max_scroll <= 0.0 || total_height <= 0.0 {
            return None;
        }

        let view_ratio = bounds.height / total_height;
        let scrollbar_height = (bounds.height * view_ratio).max(self.tokens.size(30.0));

        let scroll_ratio = if max_scroll > 0.0 {
            state.scroll_offset / max_scroll
        } else {
            0.0
        };
        let available_track = bounds.height - scrollbar_height;
        let scrollbar_y = scroll_ratio * available_track;

        Some(Rectangle {
            x: bounds.x + bounds.width - self.tokens.size(6.0) - self.tokens.space(2.0),
            y: bounds.y + scrollbar_y,
            width: self.tokens.size(6.0),
            height: scrollbar_height,
        })
    }

    fn draw_scrollbar(
        &self,
        renderer: &mut Renderer,
        bounds: Rectangle,
        state: &VirtualListState,
        is_hovered: bool,
        is_dragging: bool,
    ) {
        if let Some(scrollbar_bounds) = self.calculate_scrollbar_bounds(bounds, state) {
            let alpha = if is_dragging {
                0.6
            } else if is_hovered {
                0.5
            } else {
                0.3
            };

            renderer.fill_quad(
                renderer::Quad {
                    bounds: scrollbar_bounds,
                    border: iced::Border {
                        radius: self.tokens.size(3.0).into(),
                        width: 0.0,
                        color: Color::TRANSPARENT,
                    },
                    shadow: iced::Shadow::default(),
                    snap: true,
                },
                crate::theme::white(alpha),
            );
        }
    }
}

impl<'a, Message, Theme, Renderer, Key> From<VirtualList<'a, Message, Theme, Renderer, Key>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
    Key: Eq + Hash + Clone + 'static,
{
    fn from(list: VirtualList<'a, Message, Theme, Renderer, Key>) -> Self {
        Element::new(list)
    }
}

#[cfg(test)]
mod tests {
    use super::{VirtualList, VirtualListState};
    use crate::responsive::UiTokens;
    use iced::advanced::Shell;
    use iced::advanced::layout::{self, Layout};
    use iced::advanced::renderer;
    use iced::advanced::widget::{Tree, Widget};
    use iced::mouse::{self, Cursor};
    use iced::time::Instant;
    use iced::{Background, Element, Event, Length, Point, Rectangle, Size, Transformation};
    use iced_runtime::UserInterface;
    use iced_runtime::user_interface::Cache;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct EventProbe {
        redraw_count: Rc<Cell<usize>>,
        pointer_count: Rc<Cell<usize>>,
    }

    #[derive(Default)]
    struct LayerRecordingRenderer {
        layers: Vec<Rectangle>,
    }

    impl renderer::Renderer for LayerRecordingRenderer {
        fn start_layer(&mut self, bounds: Rectangle) {
            self.layers.push(bounds);
        }

        fn end_layer(&mut self) {}

        fn start_transformation(&mut self, _transformation: Transformation) {}

        fn end_transformation(&mut self) {}

        fn fill_quad(&mut self, _quad: renderer::Quad, _background: impl Into<Background>) {}

        fn allocate_image(
            &self,
            _handle: &iced::advanced::image::Handle,
            _callback: impl FnOnce(
                Result<iced::advanced::image::Allocation, iced::advanced::image::Error>,
            ) + Send
            + 'static,
        ) {
        }

        fn hint(&mut self, _scale: renderer::Scale) {}

        fn scale(&self) -> Option<renderer::Scale> {
            None
        }

        fn reset(&mut self, _new_bounds: Rectangle) {}

        fn settings(&self) -> renderer::Settings {
            renderer::Settings::default()
        }
    }

    struct DrawViewportProbe {
        viewports: Rc<RefCell<Vec<Rectangle>>>,
    }

    impl Widget<(), (), LayerRecordingRenderer> for DrawViewportProbe {
        fn size(&self) -> Size<Length> {
            Size::new(Length::Fill, Length::Fixed(50.0))
        }

        fn layout(
            &mut self,
            _tree: &mut Tree,
            _renderer: &LayerRecordingRenderer,
            limits: &layout::Limits,
        ) -> layout::Node {
            layout::Node::new(limits.resolve(Length::Fill, Length::Fixed(50.0), Size::ZERO))
        }

        fn draw(
            &self,
            _tree: &Tree,
            _renderer: &mut LayerRecordingRenderer,
            _theme: &(),
            _style: &renderer::Style,
            _layout: Layout<'_>,
            _cursor: Cursor,
            viewport: &Rectangle,
        ) {
            self.viewports.borrow_mut().push(*viewport);
        }
    }

    impl<Message> Widget<Message, (), ()> for EventProbe {
        fn size(&self) -> Size<Length> {
            Size::new(Length::Fill, Length::Fixed(50.0))
        }

        fn layout(
            &mut self,
            _tree: &mut Tree,
            _renderer: &(),
            limits: &layout::Limits,
        ) -> layout::Node {
            layout::Node::new(limits.resolve(Length::Fill, Length::Fixed(50.0), Size::ZERO))
        }

        fn update(
            &mut self,
            _tree: &mut Tree,
            event: &Event,
            _layout: Layout<'_>,
            _cursor: Cursor,
            _renderer: &(),
            _shell: &mut Shell<'_, Message>,
            _viewport: &Rectangle,
        ) {
            if matches!(
                event,
                Event::Window(iced::window::Event::RedrawRequested(_))
            ) {
                self.redraw_count.set(self.redraw_count.get() + 1);
            }
            if matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            ) {
                self.pointer_count.set(self.pointer_count.get() + 1);
            }
        }

        fn draw(
            &self,
            _tree: &Tree,
            _renderer: &mut (),
            _theme: &(),
            _style: &renderer::Style,
            _layout: Layout<'_>,
            _cursor: Cursor,
            _viewport: &Rectangle,
        ) {
        }
    }

    #[test]
    fn visible_range_includes_overscan_and_stays_within_item_count() {
        let mut state = VirtualListState::new(20, 50.0);
        state.update(20, 50.0, 200.0);

        assert_eq!(state.visible_range(), (0, 13));

        state.scroll_offset = 500.0;
        assert_eq!(state.visible_range(), (2, 20));
    }

    #[test]
    fn visible_range_has_bounded_bootstrap_before_layout() {
        let state = VirtualListState::new(100, 50.0);

        assert_eq!(state.visible_range(), (0, 18));
    }

    #[test]
    fn immediate_scrolling_and_jumps_are_clamped() {
        let mut state = VirtualListState::new(20, 50.0);
        state.update(20, 50.0, 200.0);

        assert_eq!(state.scroll_by_immediate(120.0), 120.0);
        assert_eq!(state.scroll_offset, 120.0);

        assert_eq!(state.scroll_by_immediate(-500.0), -120.0);
        assert_eq!(state.scroll_offset, 0.0);

        state.jump_to(f32::MAX);
        assert_eq!(state.scroll_offset, 800.0);
    }

    #[test]
    fn redraw_events_reach_visible_children() {
        let redraw_count = Rc::new(Cell::new(0));
        let probe_count = Rc::clone(&redraw_count);
        let list: VirtualList<'_, (), (), ()> =
            VirtualList::new(1, 50.0, UiTokens::default(), move |_| {
                Element::new(EventProbe {
                    redraw_count: Rc::clone(&probe_count),
                    pointer_count: Rc::new(Cell::new(0)),
                })
            })
            .scrollbar(false);
        let mut renderer = ();
        let mut user_interface = UserInterface::build(
            list,
            Size::new(200.0, 100.0),
            Cache::default(),
            &mut renderer,
        );
        let redraw = Event::Window(iced::window::Event::RedrawRequested(Instant::now()));
        let window = iced::window::Headless;
        let waker = iced::advanced::shell::Waker::noop();
        let mut messages = iced::advanced::shell::Bus::new();

        user_interface.update(
            &window,
            &waker,
            &[redraw],
            Cursor::Unavailable,
            &mut renderer,
            &mut messages,
        );

        assert_eq!(redraw_count.get(), 1);
    }

    #[test]
    fn initial_redraw_publishes_the_visible_range() {
        let list: VirtualList<'_, (usize, usize), (), ()> =
            VirtualList::new(100, 50.0, UiTokens::default(), move |_| {
                Element::new(EventProbe {
                    redraw_count: Rc::new(Cell::new(0)),
                    pointer_count: Rc::new(Cell::new(0)),
                })
            })
            .scrollbar(false)
            .on_visible_range(|range| range);
        let mut renderer = ();
        let mut user_interface = UserInterface::build(
            list,
            Size::new(200.0, 100.0),
            Cache::default(),
            &mut renderer,
        );
        let redraw = Event::Window(iced::window::Event::RedrawRequested(Instant::now()));
        let window = iced::window::Headless;
        let waker = iced::advanced::shell::Waker::noop();
        let mut messages = iced::advanced::shell::Bus::new();

        user_interface.update(
            &window,
            &waker,
            &[redraw],
            Cursor::Unavailable,
            &mut renderer,
            &mut messages,
        );

        assert_eq!(messages.into_iter().collect::<Vec<_>>(), vec![(0, 11)]);
    }

    #[test]
    fn pointer_events_only_reach_the_hit_row() {
        let pointer_counts = [Rc::new(Cell::new(0)), Rc::new(Cell::new(0))];
        let probe_counts = pointer_counts.clone();
        let list: VirtualList<'_, (), (), ()> =
            VirtualList::new(2, 50.0, UiTokens::default(), move |index| {
                Element::new(EventProbe {
                    redraw_count: Rc::new(Cell::new(0)),
                    pointer_count: Rc::clone(&probe_counts[index]),
                })
            })
            .scrollbar(false);
        let mut renderer = ();
        let mut user_interface = UserInterface::build(
            list,
            Size::new(200.0, 100.0),
            Cache::default(),
            &mut renderer,
        );
        let click = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let window = iced::window::Headless;
        let waker = iced::advanced::shell::Waker::noop();
        let mut messages = iced::advanced::shell::Bus::new();

        user_interface.update(
            &window,
            &waker,
            &[click],
            Cursor::Available(Point::new(20.0, 75.0)),
            &mut renderer,
            &mut messages,
        );

        assert_eq!(pointer_counts[0].get(), 0);
        assert_eq!(pointer_counts[1].get(), 1);
    }

    #[test]
    fn drawing_layers_and_children_use_the_parent_viewport_intersection() {
        let child_viewports = Rc::new(RefCell::new(Vec::new()));
        let observed_viewports = Rc::clone(&child_viewports);
        let mut list: VirtualList<'_, (), (), LayerRecordingRenderer> =
            VirtualList::new(3, 50.0, UiTokens::default(), move |_| {
                Element::new(DrawViewportProbe {
                    viewports: Rc::clone(&observed_viewports),
                })
            });
        let mut tree = Tree::new(&list as &dyn Widget<(), (), LayerRecordingRenderer>);
        let mut renderer = LayerRecordingRenderer::default();
        let limits = layout::Limits::new(Size::ZERO, Size::new(200.0, 100.0));
        let node = list.layout(&mut tree, &renderer, &limits);
        let viewport = Rectangle {
            x: 0.0,
            y: 20.0,
            width: 200.0,
            height: 60.0,
        };

        list.draw(
            &tree,
            &mut renderer,
            &(),
            &renderer::Style::default(),
            Layout::new(&node),
            Cursor::Unavailable,
            &viewport,
        );

        assert_eq!(renderer.layers, vec![viewport, viewport]);
        assert_eq!(child_viewports.borrow().as_slice(), &[viewport, viewport]);
    }
}
