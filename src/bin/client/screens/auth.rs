use iced::{
  Border, Color, Element,
  Length::{self, Fill, FillPortion},
  Pixels, Theme,
  border::{self, color},
  widget::{
    button, button::Style as ButtonStyle, column, container, container::Style as ContainerStyle,
    row, space, text_input, text_input::Style as TextStyle,
  },
};

use crate::SPACE_GRID;

// -------------------- MODEL --------------------

#[derive(Default)]
pub struct Model {
  email_input: String,
  remember_me_checked: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
  UserChangedLoginInput(String),
  UserSubmittedLogin,
  UserClickedRegister,
  UserToggledRememberMe,
}

// -------------------- VIEW --------------------

pub fn view<'a>(model: &Model) -> Element<'a, Message> {
  row![login_card(model), hero()]
    .spacing(Pixels(SPACE_GRID.into()))
    .width(Fill)
    .height(Fill)
    .into()
}

fn login_card<'a>(model: &Model) -> Element<'a, Message> {
  container(
    container(
      column![
        column![
          text_input("Email", &model.email_input)
            .on_input(Message::UserChangedLoginInput)
            .style(|theme: &Theme, status| {
              let palette = theme.extended_palette();

              TextStyle {
                border: Border {
                  width: 0.0,
                  ..Border::default()
                },
                background: iced::Background::Color(palette.background.stronger.color),
                value: palette.background.stronger.text,
                placeholder: palette.background.strong.text,
                ..text_input::default(theme, status)
              }
            }),
          container(space())
            .width(Fill)
            .height(2)
            .style(container::bordered_box)
        ],
        button(container("Login").center_x(Fill))
          .width(Fill)
          .style(|_theme: &Theme, _status| {
            ButtonStyle {
              background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
              text_color: Color::from_rgb(0.2, 0.2, 0.2),
              border: Border {
                radius: border::Radius::new(Pixels(SPACE_GRID.into()) / 2),
                ..Border::default()
              },
              ..ButtonStyle::default()
            }
          })
      ]
      .spacing(Pixels(SPACE_GRID.into()) * 2.)
      .width(400),
    )
    .center(Fill),
  )
  .style(|theme: &Theme| {
    let palette = theme.extended_palette();

    ContainerStyle {
      background: Some(palette.background.stronger.color.into()),
      text_color: Some(palette.background.stronger.text),
      border: Border {
        width: 1.0,
        radius: 5.0.into(),
        color: palette.background.weak.color,
      },
      ..ContainerStyle::default()
    }
  })
  .padding(SPACE_GRID * 4)
  .width(FillPortion(9))
  .height(Fill)
  .into()
}

fn hero<'a>() -> Element<'a, Message> {
  iced::widget::container(
    iced::widget::image(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/src/bin/client/images/bg-login.jpg"
    ))
    .content_fit(iced::ContentFit::Fill),
  )
  .style(container::bordered_box)
  .width(FillPortion(15))
  .height(Fill)
  .into()
}

// -------------------- UPDATE --------------------

pub fn update(model: &mut Model, message: Message) {
  match message {
    Message::UserChangedLoginInput(new) => model.email_input = new,
    Message::UserSubmittedLogin => todo!(),
    Message::UserClickedRegister => todo!(),
    Message::UserToggledRememberMe => todo!(),
  }
}
