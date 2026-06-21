use google_material_symbols::GoogleMaterialSymbols;
use iced::{
  Border, Element, Font,
  Length::{self, FillPortion},
  Task, Theme,
  font::Weight,
  widget::{button, column, container, keyed::column, row, rule, space, text},
};

use crate::{
  MATERIAL, SOURCE_SANS_REGULAR, SPACE_GRID,
  colors::{NeutralsExt, TextExt},
  icon,
};

#[derive(Clone)]
pub enum Message {
  None,
}

#[derive(Default)]
enum View {
  #[default]
  Voice,
}

#[derive(Default)]
pub struct Model {
  view: View,
}

pub fn update(model: &mut Model, msg: Message) -> Task<Message> {
  match msg {
    Message::None => todo!(),
  }
}

pub fn view<'a>(model: &'a Model) -> Element<'a, Message> {
  row![
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
        .on_press(Message::None)
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
    container(text(""))
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
      .padding([SPACE_GRID * 6, SPACE_GRID])
  ]
  .width(Length::Fill)
  .height(Length::Fill)
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
