use google_material_symbols::GoogleMaterialSymbols;
use iced::{
  Border, Element, Font,
  Length::{self, FillPortion},
  Task, Theme,
  font::Weight,
  widget::{
    button, column, container, keyed::column, pick_list, progress_bar, row, rule, slider, space,
    stack, text,
  },
};

use crate::{
  MATERIAL, SOURCE_SANS_REGULAR, SPACE_GRID,
  colors::TextExt,
  icon,
  voice_settings::VoiceSettings,
  webrtc_stream::{MicMonitor, list_input_devices, list_output_devices},
};

// The gate threshold is a linear RMS value, but voice levels bunch up near the
// bottom of 0..1 — speech RMS typically lives around 0.01..0.05 — so an
// amplitude curve (even squared) leaves the slider and meter pinned to the
// bottom. Instead the slider works in a decibel "display" space, the way audio
// meters do: display maps linearly across GATE_FLOOR_DB..0 dB, which spreads
// the quiet, useful range over the whole 0..1 travel. Display 1.0 → 0 dB
// (threshold 1.0, cuts all audio); display 0 → 0 (gate off).
const GATE_FLOOR_DB: f32 = -60.0;

fn display_to_threshold(display: f32) -> f32 {
  if display <= 0.0 {
    return 0.0; // gate off
  }
  let db = GATE_FLOOR_DB * (1.0 - display.min(1.0));
  10f32.powf(db / 20.0)
}
fn threshold_to_display(threshold: f32) -> f32 {
  if threshold <= 0.0 {
    return 0.0;
  }
  let db = 20.0 * threshold.log10();
  (1.0 - db / GATE_FLOOR_DB).clamp(0.0, 1.0)
}
fn level_to_display(rms: f32) -> f32 {
  if rms <= 0.0 {
    return 0.0;
  }
  let db = 20.0 * rms.log10();
  (1.0 - db / GATE_FLOOR_DB).clamp(0.0, 1.0)
}

#[derive(Clone)]
pub enum Message {
  /// Inert (the lone "Voice" tab — there's only one view today).
  None,
  /// Live while dragging the gate slider; applied to the handle each tick.
  NoiseGateChanged(f32),
  /// Slider released — persist (so we don't thrash the disk mid-drag).
  NoiseGateReleased,
  InputDeviceSelected(String),
  OutputDeviceSelected(String),
  /// Periodic poll of the live mic-level meter (driven by a subscription).
  Tick,
  /// Handled in `main` (needs top-level model access).
  LogOut,
  /// Back to chat (close button / Esc). Handled in `main`.
  Close,
}

#[derive(Default)]
enum View {
  #[default]
  Voice,
}

pub struct Model {
  view: View,
  gate_threshold: f32,
  input_device: Option<String>,
  output_device: Option<String>,
  input_devices: Vec<String>,
  output_devices: Vec<String>,
  // standalone mic meter for calibrating the gate on this screen; the latest
  // sampled level is mirrored into `input_level` on each Tick.
  mic_monitor: Option<MicMonitor>,
  input_level: f32,
}

impl Default for Model {
  fn default() -> Self {
    // mirror the persisted settings + enumerate the devices to offer.
    let settings = VoiceSettings::load();
    let mic_monitor = Some(MicMonitor::start(settings.input_device.clone()));
    Self {
      view: View::Voice,
      gate_threshold: settings.gate_threshold,
      input_device: settings.input_device,
      output_device: settings.output_device,
      input_devices: list_input_devices(),
      output_devices: list_output_devices(),
      mic_monitor,
      input_level: 0.0,
    }
  }
}

impl Model {
  fn persist(&self) {
    VoiceSettings {
      gate_threshold: self.gate_threshold,
      input_device: self.input_device.clone(),
      output_device: self.output_device.clone(),
    }
    .save();
  }
}

pub fn update(model: &mut Model, msg: Message) -> Task<Message> {
  match msg {
    Message::NoiseGateChanged(threshold) => model.gate_threshold = threshold,
    Message::NoiseGateReleased => model.persist(),
    Message::InputDeviceSelected(name) => {
      model.input_device = Some(name);
      // re-point the meter at the newly selected device.
      model.mic_monitor = Some(MicMonitor::start(model.input_device.clone()));
      model.persist();
    }
    Message::OutputDeviceSelected(name) => {
      model.output_device = Some(name);
      model.persist();
    }
    Message::Tick => {
      if let Some(monitor) = &model.mic_monitor {
        model.input_level = monitor.level();
      }
    }
    // intercepted in `main`; never delegated here, but keep the match total.
    Message::None | Message::LogOut | Message::Close => {}
  }
  Task::none()
}

pub fn view<'a>(model: &'a Model) -> Element<'a, Message> {
  let body = row![
    container(
      column![
        view_section(
          "APP SETTINGS",
          vec![view_tab_selector(
            "Voice",
            Message::None,
            matches!(model.view, View::Voice)
          )]
        ),
        button(row![
          text("Log Out")
            .font(Font {
              weight: Weight::Semibold,
              ..SOURCE_SANS_REGULAR
            })
            .size(16),
          space::horizontal(),
          icon(GoogleMaterialSymbols::Logout)
            .font(Font {
              weight: Weight::Semibold,
              ..MATERIAL
            })
            .size(16),
        ],)
        .style(|theme: &Theme, status| {
          let palette = theme.extended_palette();
          let pair = match status {
            button::Status::Active => palette.danger.base,
            button::Status::Hovered => palette.danger.strong,
            button::Status::Pressed => palette.danger.strong,
            button::Status::Disabled => palette.background.base,
          };
          button::Style {
            background: None,
            text_color: pair.color,
            border: Border::default().rounded((SPACE_GRID / 2) as u32),
            ..button::Style::default()
          }
        })
        .on_press(Message::LogOut)
        .width(Length::Fill)
        .padding(SPACE_GRID),
      ]
      .width(180)
    )
    .style(|theme: &Theme| {
      let palette = theme.extended_palette();
      container::Style {
        background: Some(palette.background.weakest.color.into()),
        ..container::Style::default()
      }
    })
    .height(Length::Fill)
    .align_right(FillPortion(4))
    .height(Length::Fill)
    .padding([SPACE_GRID * 6, SPACE_GRID]),
    container(view_voice_settings(model))
      .style(|theme: &Theme| {
        let palette = theme.extended_palette();
        container::Style {
          background: Some(palette.background.strong.color.into()),
          text_color: Some(palette.background.strong.text),
          ..container::Style::default()
        }
      })
      .width(Length::FillPortion(7))
      .height(Length::Fill)
      .padding([SPACE_GRID * 6, SPACE_GRID * 4])
  ]
  .width(Length::Fill)
  .height(Length::Fill);

  // persistent circular close button pinned to the top-right of the whole view.
  stack![
    body,
    container(view_close_button())
      .width(Length::Fill)
      .align_x(iced::alignment::Horizontal::Right)
      .padding(SPACE_GRID * 2),
  ]
  .width(Length::Fill)
  .height(Length::Fill)
  .into()
}

fn view_close_button<'a>() -> Element<'a, Message> {
  button(
    icon(GoogleMaterialSymbols::Close)
      .font(Font {
        weight: Weight::Semibold,
        ..MATERIAL
      })
      .size(18),
  )
  .on_press(Message::Close)
  .padding(SPACE_GRID)
  .style(|theme: &Theme, status| {
    let palette = theme.extended_palette();
    let pair = match status {
      button::Status::Hovered | button::Status::Pressed => palette.background.strongest,
      _ => palette.background.strong,
    };
    button::Style {
      background: Some(pair.color.into()),
      text_color: pair.text,
      border: Border::default().rounded(100),
      ..button::Style::default()
    }
  })
  .into()
}

fn view_voice_settings<'a>(model: &'a Model) -> Element<'a, Message> {
  column![
    text("Voice Settings").size(24).font(Font {
      weight: Weight::Bold,
      ..SOURCE_SANS_REGULAR
    }),
    view_noise_gate(model),
    column![
      view_device_picker(
        "Input Device",
        &model.input_devices,
        model.input_device.clone(),
        Message::InputDeviceSelected,
      ),
      view_device_picker(
        "Output Device",
        &model.output_devices,
        model.output_device.clone(),
        Message::OutputDeviceSelected,
      )
    ]
    .spacing((SPACE_GRID * 2) as u32),
  ]
  .spacing((SPACE_GRID * 3) as u32)
  .into()
}

fn view_noise_gate<'a>(model: &'a Model) -> Element<'a, Message> {
  // the slider works in display (perceptual) space; the meter shares the curve
  // so the live level lines up under the threshold handle.
  let display = threshold_to_display(model.gate_threshold);
  let level = level_to_display(model.input_level);
  let open = model.input_level > model.gate_threshold && model.gate_threshold > 0.0;

  let label = if model.gate_threshold <= 0.0 {
    "Off".to_string()
  } else {
    format!("{:.0}%", display * 100.0)
  };

  column![
    row![
      text("Noise Gate").font(Font {
        weight: Weight::Semibold,
        ..SOURCE_SANS_REGULAR
      }),
      space::horizontal(),
      text(label).style(|theme: &Theme| text::Style {
        color: Some(theme.text().subtext0)
      }),
    ],
    slider(0.0..=1.0, display, |d| Message::NoiseGateChanged(
      display_to_threshold(d)
    ))
    .step(0.01f32)
    .on_release(Message::NoiseGateReleased),
    // live mic meter on the same scale. Green when the gate would be open
    // (level above threshold), muted otherwise — so the user can see where to
    // park the handle relative to their background noise.
    progress_bar(0.0..=1.0, level)
      .girth(Length::Fixed(6.0))
      .style(move |theme: &Theme| {
        let palette = theme.extended_palette();
        progress_bar::Style {
          background: palette.background.weak.color.into(),
          bar: if open {
            palette.success.base.color.into()
          } else {
            palette.background.strongest.color.into()
          },
          border: Border::default().rounded(3),
        }
      }),
    text("Mic stays muted until your level (the bar) rises above the handle.")
      .size(13)
      .style(|theme: &Theme| text::Style {
        color: Some(theme.text().overlay1)
      }),
  ]
  .spacing(SPACE_GRID as u32)
  .into()
}

fn view_device_picker<'a>(
  label: &'a str,
  options: &'a [String],
  selected: Option<String>,
  on_select: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
  column![
    text(label).font(Font {
      weight: Weight::Semibold,
      ..SOURCE_SANS_REGULAR
    }),
    pick_list(options, selected, on_select)
      .placeholder("System default")
      .style(|theme, status| pick_list::Style {
        border: Border::default().rounded((SPACE_GRID / 2) as u32),
        ..pick_list::default(theme, status)
      })
      .width(Length::Fill),
  ]
  .spacing(SPACE_GRID as u32)
  .into()
}

fn view_section<'a>(title: &'a str, children: Vec<Element<'a, Message>>) -> Element<'a, Message> {
  column![
    text(title)
      .style(|theme: &Theme| text::Style {
        color: Some(theme.text().overlay0)
      })
      .size(14)
      .font(Font {
        weight: Weight::Bold,
        ..SOURCE_SANS_REGULAR
      }),
    column::Column::with_children(
      children
        .into_iter()
        .enumerate()
        .collect::<Vec<(usize, Element<'a, Message>)>>()
    )
    .spacing(1),
    container(rule::horizontal(1).style(|theme: &Theme| rule::Style {
      color: theme.extended_palette().background.strong.color,
      ..rule::default(theme)
    }))
    .width(Length::Fill)
    .padding(SPACE_GRID / 2),
  ]
  .spacing((SPACE_GRID) as u32)
  .into()
}

fn view_tab_selector<'a>(name: &'a str, on_press: Message, selected: bool) -> Element<'a, Message> {
  button(
    text(name)
      .style(move |theme: &Theme| text::Style {
        color: if selected {
          Some(theme.text().text)
        } else {
          Some(theme.text().subtext0)
        },
      })
      .font(Font {
        weight: Weight::Semibold,
        ..SOURCE_SANS_REGULAR
      })
      .size(16),
  )
  .style(|theme: &Theme, status| {
    let palette = theme.extended_palette();
    let pair = match status {
      button::Status::Active => palette.background.strongest,
      button::Status::Hovered => palette.background.strong,
      button::Status::Pressed => palette.background.strong,
      button::Status::Disabled => palette.background.base,
    };
    button::Style {
      background: Some(pair.color.into()),
      text_color: pair.text,
      border: Border::default().rounded((SPACE_GRID / 2) as u32),
      ..button::Style::default()
    }
  })
  .on_press(on_press)
  .width(Length::Fill)
  .padding(SPACE_GRID)
  .into()
}
