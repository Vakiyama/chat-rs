//! A right-click context menu.
//!
//! Vendored near-verbatim from `iced_aw` 0.14.1 (`widget::context_menu` plus its
//! `widget::overlay::context_menu`) with **one behavioral fix**: the upstream
//! overlay closed on *any* left-button release, which made interactive content
//! unusable — finishing a volume-slider drag or clicking a button releases the
//! mouse and so dismissed the menu instantly. Here we only close on a release
//! that lands *outside* the menu content (see `OVERLAY FIX` below). Opening on
//! right-click, dismiss-on-outside-click, and dismiss-on-Escape are unchanged.
//!
//! Styling reuses `iced_aw`'s public context-menu catalog, so it themes
//! identically to the rest of the app. Trimmed of the upstream `force_open` test
//! hook and unit tests, which we don't use.

use iced_aw::style::context_menu::{Catalog, Style};
use iced_aw::style::status::{Status, StyleFn};
use iced_core::{
  Border, Clipboard, Color, Element, Event, Layout, Length, Point, Rectangle, Shell, Size, Vector,
  Widget, keyboard,
  layout::{Limits, Node},
  mouse::{self, Button, Cursor},
  overlay, renderer, touch,
  widget::{Operation, Tree, tree},
  window,
};

/// A context menu.
#[allow(missing_debug_implementations)]
pub struct ContextMenu<
  'a,
  Overlay,
  Message,
  Theme = iced_widget::Theme,
  Renderer = iced_widget::Renderer,
> where
  Overlay: Fn() -> Element<'a, Message, Theme, Renderer>,
  Message: Clone,
  Renderer: renderer::Renderer,
  Theme: Catalog,
{
  /// The underlying element.
  underlay: Element<'a, Message, Theme, Renderer>,
  /// The content of the overlay.
  overlay: Overlay,
  overlay_instance: Option<Element<'a, Message, Theme, Renderer>>,
  /// The style of the [`ContextMenu`].
  class: Theme::Class<'a>,
  /// When set, a left-click *inside* the menu content closes it (after the
  /// clicked control receives the event). Off by default so interactive menus
  /// (e.g. a volume slider) stay open across drags; on for one-shot action
  /// menus that should dismiss as soon as an item is chosen.
  close_on_release: bool,
}

impl<'a, Overlay, Message, Theme, Renderer> ContextMenu<'a, Overlay, Message, Theme, Renderer>
where
  Overlay: Fn() -> Element<'a, Message, Theme, Renderer>,
  Message: Clone,
  Renderer: renderer::Renderer,
  Theme: Catalog,
{
  /// Creates a new [`ContextMenu`].
  ///
  /// `underlay`: the element shown normally; right-click it to open the menu.
  /// `overlay`: builds the menu content shown on right-click.
  pub fn new<U>(underlay: U, overlay: Overlay) -> Self
  where
    U: Into<Element<'a, Message, Theme, Renderer>>,
  {
    ContextMenu {
      underlay: underlay.into(),
      overlay,
      overlay_instance: None,
      class: Theme::default(),
      close_on_release: false,
    }
  }

  /// Closes the menu when a left-click lands inside its content (the clicked
  /// control still receives the event first). Use for one-shot action menus.
  #[must_use]
  pub fn close_on_release(mut self, close: bool) -> Self {
    self.close_on_release = close;
    self
  }

  /// Sets the style of the [`ContextMenu`].
  #[must_use]
  pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
  where
    Theme::Class<'a>: From<StyleFn<'a, Theme, Style>>,
  {
    self.class = (Box::new(style) as StyleFn<'a, Theme, Style>).into();
    self
  }

  /// Sets the class of the [`ContextMenu`].
  #[must_use]
  pub fn class(mut self, class: impl Into<Theme::Class<'a>>) -> Self {
    self.class = class.into();
    self
  }
}

impl<'a, Content, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
  for ContextMenu<'a, Content, Message, Theme, Renderer>
where
  Content: 'a + Fn() -> Element<'a, Message, Theme, Renderer>,
  Message: 'a + Clone,
  Renderer: 'a + renderer::Renderer,
  Theme: Catalog,
{
  fn size(&self) -> Size<Length> {
    self.underlay.as_widget().size()
  }

  fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> Node {
    self
      .underlay
      .as_widget_mut()
      .layout(&mut tree.children[0], renderer, limits)
  }

  fn draw(
    &self,
    state: &Tree,
    renderer: &mut Renderer,
    theme: &Theme,
    style: &renderer::Style,
    layout: Layout<'_>,
    cursor: Cursor,
    viewport: &Rectangle,
  ) {
    self.underlay.as_widget().draw(
      &state.children[0],
      renderer,
      theme,
      style,
      layout,
      cursor,
      viewport,
    );
  }

  fn tag(&self) -> tree::Tag {
    tree::Tag::of::<State>()
  }

  fn state(&self) -> tree::State {
    tree::State::new(State::new())
  }

  fn children(&self) -> Vec<Tree> {
    let overlay_tree = self
      .overlay_instance
      .as_ref()
      .map_or_else(Tree::empty, Tree::new);
    vec![Tree::new(&self.underlay), overlay_tree]
  }

  fn diff(&self, tree: &mut Tree) {
    tree.children[0].diff(&self.underlay);
    if let Some(overlay) = self.overlay_instance.as_ref() {
      tree.children[1].diff(overlay);
    }
  }

  fn operate<'b>(
    &'b mut self,
    state: &'b mut Tree,
    layout: Layout<'_>,
    renderer: &Renderer,
    operation: &mut dyn Operation<()>,
  ) {
    let s: &mut State = state.state.downcast_mut();

    if s.show {
      let content = self.overlay_instance.get_or_insert_with(&self.overlay);
      state.children[1].diff(&*content);

      content
        .as_widget_mut()
        .operate(&mut state.children[1], layout, renderer, operation);
    } else {
      self.overlay_instance = None;
      self
        .underlay
        .as_widget_mut()
        .operate(&mut state.children[0], layout, renderer, operation);
    }
  }

  fn update(
    &mut self,
    state: &mut Tree,
    event: &Event,
    layout: Layout<'_>,
    cursor: Cursor,
    renderer: &Renderer,
    clipboard: &mut dyn Clipboard,
    shell: &mut Shell<'_, Message>,
    viewport: &Rectangle,
  ) {
    if *event == Event::Mouse(mouse::Event::ButtonPressed(Button::Right)) {
      let bounds = layout.bounds();

      if cursor.is_over(bounds) {
        let s: &mut State = state.state.downcast_mut();
        s.cursor_position = cursor.position().unwrap_or_default();
        s.show = !s.show;

        if !s.show {
          self.overlay_instance = None;
        }

        shell.capture_event();
        shell.request_redraw();
      }
    }

    self.underlay.as_widget_mut().update(
      &mut state.children[0],
      event,
      layout,
      cursor,
      renderer,
      clipboard,
      shell,
      viewport,
    );
  }

  fn mouse_interaction(
    &self,
    state: &Tree,
    layout: Layout<'_>,
    cursor: Cursor,
    viewport: &Rectangle,
    renderer: &Renderer,
  ) -> mouse::Interaction {
    self.underlay.as_widget().mouse_interaction(
      &state.children[0],
      layout,
      cursor,
      viewport,
      renderer,
    )
  }

  fn overlay<'b>(
    &'b mut self,
    tree: &'b mut Tree,
    layout: Layout<'b>,
    renderer: &Renderer,
    viewport: &Rectangle,
    translation: Vector,
  ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
    let s: &mut State = tree.state.downcast_mut();

    if !s.show {
      self.overlay_instance = None;
      return self.underlay.as_widget_mut().overlay(
        &mut tree.children[0],
        layout,
        renderer,
        viewport,
        translation,
      );
    }

    let position = s.cursor_position;
    let content = self.overlay_instance.get_or_insert_with(&self.overlay);
    tree.children[1].diff(&*content);
    Some(
      ContextMenuOverlay::new(
        position + translation,
        &mut tree.children[1],
        content,
        &self.class,
        s,
        self.close_on_release,
      )
      .overlay(),
    )
  }
}

impl<'a, Content, Message, Theme, Renderer> From<ContextMenu<'a, Content, Message, Theme, Renderer>>
  for Element<'a, Message, Theme, Renderer>
where
  Content: 'a + Fn() -> Self,
  Message: 'a + Clone,
  Renderer: 'a + renderer::Renderer,
  Theme: 'a + Catalog,
{
  fn from(modal: ContextMenu<'a, Content, Message, Theme, Renderer>) -> Self {
    Element::new(modal)
  }
}

/// The state shared between the [`ContextMenu`] and its overlay.
#[derive(Debug, Default)]
struct State {
  /// The visibility of the [`ContextMenu`] overlay.
  pub show: bool,
  /// Where the overlay is anchored (the cursor position at open time).
  pub cursor_position: Point,
}

impl State {
  const fn new() -> Self {
    Self {
      show: false,
      cursor_position: Point::ORIGIN,
    }
  }
}

/// The overlay of the [`ContextMenu`].
struct ContextMenuOverlay<'a, 'b, Message, Theme, Renderer>
where
  Message: 'a + Clone,
  Renderer: 'a + renderer::Renderer,
  Theme: Catalog,
  'b: 'a,
{
  /// The position the overlay is anchored at.
  position: Point,
  /// The state of the overlay's content tree.
  tree: &'a mut Tree,
  /// The content of the overlay.
  content: &'a mut Element<'b, Message, Theme, Renderer>,
  /// The style of the overlay.
  class: &'a Theme::Class<'b>,
  /// The state shared with the [`ContextMenu`].
  state: &'a mut State,
  /// Whether a left-click inside the content should dismiss the menu.
  close_on_release: bool,
}

impl<'a, 'b, Message, Theme, Renderer> ContextMenuOverlay<'a, 'b, Message, Theme, Renderer>
where
  Message: Clone,
  Renderer: renderer::Renderer,
  Theme: 'a + Catalog,
  'b: 'a,
{
  fn new(
    position: Point,
    tree: &'a mut Tree,
    content: &'a mut Element<'b, Message, Theme, Renderer>,
    class: &'a <Theme as Catalog>::Class<'b>,
    state: &'a mut State,
    close_on_release: bool,
  ) -> Self {
    ContextMenuOverlay {
      position,
      tree,
      content,
      class,
      state,
      close_on_release,
    }
  }

  fn overlay(self) -> overlay::Element<'a, Message, Theme, Renderer> {
    overlay::Element::new(Box::new(self))
  }
}

impl<'a, 'b, Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
  for ContextMenuOverlay<'a, 'b, Message, Theme, Renderer>
where
  Message: 'a + Clone,
  Renderer: 'a + renderer::Renderer,
  Theme: 'a + Catalog,
  'b: 'a,
{
  fn layout(&mut self, renderer: &Renderer, bounds: Size) -> Node {
    let limits = Limits::new(Size::ZERO, bounds);
    let max_size = limits.max();

    let mut content = self
      .content
      .as_widget_mut()
      .layout(self.tree, renderer, &limits);

    // Try to stay inside borders
    let mut position = self.position;
    if position.x + content.size().width > bounds.width {
      position.x = f32::max(0.0, position.x - content.size().width);
    }
    if position.y + content.size().height > bounds.height {
      position.y = f32::max(0.0, position.y - content.size().height);
    }

    content.move_to_mut(position);

    Node::with_children(max_size, vec![content])
  }

  fn draw(
    &self,
    renderer: &mut Renderer,
    theme: &Theme,
    style: &renderer::Style,
    layout: Layout<'_>,
    cursor: Cursor,
  ) {
    let bounds = layout.bounds();

    let style_sheet = theme.style(self.class, Status::Active);

    let content_layout = layout
      .children()
      .next()
      .expect("widget: Layout should have a content layout.");

    // Background
    renderer.fill_quad(
      renderer::Quad {
        bounds: content_layout.bounds(),
        border: Border {
          radius: (0.0).into(),
          width: 0.0,
          color: Color::TRANSPARENT,
        },
        ..Default::default()
      },
      style_sheet.background,
    );

    // Content
    self.content.as_widget().draw(
      self.tree,
      renderer,
      theme,
      style,
      content_layout,
      cursor,
      &bounds,
    );
  }

  fn update(
    &mut self,
    event: &Event,
    layout: Layout<'_>,
    cursor: Cursor,
    renderer: &Renderer,
    clipboard: &mut dyn Clipboard,
    shell: &mut Shell<'_, Message>,
  ) {
    let layout_children = layout
      .children()
      .next()
      .expect("widget: Layout should have a content layout.");

    let mut forward_event_to_children = true;
    let mut capture_event = false;

    match &event {
      Event::Keyboard(keyboard::Event::KeyPressed { key, .. })
        if *key == keyboard::Key::Named(keyboard::key::Named::Escape) =>
      {
        self.state.show = false;
        forward_event_to_children = false;
        shell.capture_event();
        shell.request_redraw();
      }
      Event::Mouse(mouse::Event::ButtonPressed(Button::Left | Button::Right))
      | Event::Touch(touch::Event::FingerPressed { .. }) => {
        if cursor.is_over(layout_children.bounds()) {
          capture_event = true;
        } else {
          self.state.show = false;
          forward_event_to_children = false;
          shell.request_redraw();
        }
      }

      // OVERLAY FIX (the reason this widget is vendored): the upstream
      // closed on every left release. We only close when the release lands
      // *outside* the menu content, so finishing a slider drag or clicking
      // a button inside the menu keeps it open. A release inside is still
      // forwarded to the children so those controls receive it.
      Event::Mouse(mouse::Event::ButtonReleased(Button::Left)) => {
        if !cursor.is_over(layout_children.bounds()) {
          self.state.show = false;
          capture_event = true;
          shell.request_redraw();
        } else if self.close_on_release {
          // dismiss one-shot action menus once an item is chosen. We don't
          // capture, so the clicked control still receives this release and
          // emits its message.
          self.state.show = false;
          shell.request_redraw();
        }
      }

      Event::Window(window::Event::Resized { .. }) => {
        self.state.show = false;
        forward_event_to_children = false;
        capture_event = true;
        shell.request_redraw();
      }

      _ => {}
    }

    if forward_event_to_children {
      self.content.as_widget_mut().update(
        self.tree,
        event,
        layout_children,
        cursor,
        renderer,
        clipboard,
        shell,
        &layout.bounds(),
      );
    }
    if capture_event {
      shell.capture_event();
    }
  }

  fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
    let content_layout = layout
      .children()
      .next()
      .expect("widget: Layout should have a content layout.");

    self
      .content
      .as_widget_mut()
      .operate(self.tree, content_layout, renderer, operation);
  }

  fn mouse_interaction(
    &self,
    layout: Layout<'_>,
    cursor: Cursor,
    renderer: &Renderer,
  ) -> mouse::Interaction {
    let bounds = layout.bounds();

    self.content.as_widget().mouse_interaction(
      self.tree,
      layout
        .children()
        .next()
        .expect("widget: Layout should have a content layout."),
      cursor,
      &bounds,
      renderer,
    )
  }
}
