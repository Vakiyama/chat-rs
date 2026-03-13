use std::{str::FromStr, sync::Arc};

use iced::{
  Border, Color, Element,
  Length::{self, Fill, FillPortion},
  Pixels, Task, Theme,
  border::{self, color},
  widget::{
    button, button::Style as ButtonStyle, column, container, container::Style as ContainerStyle,
    float, row, space, text, text_input, text_input::Style as TextStyle,
  },
};
use resend_rs::Resend;

use crate::SPACE_GRID;
use crate::library::resend;

// -------------------- MODEL --------------------

pub enum Mode {
  Login { error_message: Option<&'static str> },
  Register { error_message: Option<&'static str> },
  Code { error_message: Option<&'static str> },
}

impl Default for Mode {
  fn default() -> Self {
    Self::Login {
      error_message: None,
    }
  }
}

#[derive(Default)]
pub struct Model {
  mode: Mode,
  email_input: String,
  remember_me_checked: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
  UserChangedLoginInput(String),
  UserSubmittedForm,
  UserNavigatedRegister,
  UserNavigatedLogin,
  UserToggledRememberMe,
  ResendSentEmail(Arc<Result<String, resend::Error>>),
}

// -------------------- UPDATE --------------------

pub fn update(model: &mut Model, message: Message, resend: Arc<Resend>) -> Task<Message> {
  match message {
    Message::UserChangedLoginInput(new) => {
      model.email_input = new;
      Task::none()
    }
    Message::UserSubmittedForm => Task::perform(
      resend::send_auth_email(model.email_input.clone(), resend),
      |response| Message::ResendSentEmail(Arc::new(response)),
    ),
    Message::UserToggledRememberMe => {
      model.remember_me_checked = !model.remember_me_checked;
      Task::none()
    }
    Message::UserNavigatedRegister => {
      model.mode = Mode::Register {
        error_message: None,
      };
      Task::none()
    }
    Message::UserNavigatedLogin => {
      model.mode = Mode::Login {
        error_message: None,
      };
      Task::none()
    }
    Message::ResendSentEmail(response) => {
      println!("{response:?}");

      let new_err_msg = match *response {
        Ok(_) => None,
        Err(resend::Error::Api(_)) => {
          Some("An error ocurred while sending your confirmation email. Please try again later.")
        }
        Err(resend::Error::EmailValidation(_)) => Some("Invalid email."),
      };

      match (&mut model.mode, &*response) {
        (Mode::Login { error_message }, Err(_)) | (Mode::Register { error_message }, Err(_)) => {
          *error_message = new_err_msg;
        }
        (Mode::Login { .. }, Ok(_)) | (Mode::Register { .. }, Ok(_)) => {
          model.mode = Mode::Code {
            error_message: None,
          }
        }
        (Mode::Code { .. }, _) => (), // ignore responses when already in code view
      };

      Task::none()
    }
  }
}

// -------------------- VIEW --------------------

pub fn view<'a>(model: &Model) -> Element<'a, Message> {
  row![left_card(model), hero()]
    .spacing(Pixels(SPACE_GRID.into()))
    .width(Fill)
    .height(Fill)
    .into()
}

fn left_card<'a>(model: &Model) -> Element<'a, Message> {
  container(
    container(
      column![
        render_left_content(model),
        button(container("Login").center_x(Fill))
          .on_press(Message::UserSubmittedForm)
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
          }),
        {
          if let Mode::Login {
            error_message: Some(err),
          } = model.mode
          {
            float(text(err).color(Color::from_rgb(1., 0.5, 0.5)).size(12))
          } else {
            float(text("").height(0))
          }
        },
      ]
      .spacing(Pixels(SPACE_GRID.into()))
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

fn render_left_content<'a>(model: &Model) -> Element<'a, Message> {
  match model.mode {
    Mode::Login { .. } => login_content(model),
    Mode::Register { .. } => todo!(),
    Mode::Code { .. } => code_input_content(model),
  }
}

fn code_input_content<'a>(model: &Model) -> Element<'a, Message> {
  column![
    text_input("Enter Code", &model.email_input)
      .on_input(Message::UserChangedLoginInput)
      .on_submit(Message::UserSubmittedForm)
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
      .style(container::bordered_box),
    row![
      container("").width(Fill),
      button(
        text("Back to Login")
          .align_x(text::Alignment::Right)
          .size(12)
          .color(Color::from_rgba(1., 1., 1., 0.8))
      )
      .on_press(Message::UserNavigatedLogin)
      .style(|theme: &Theme, _status| {
        let palette = theme.extended_palette();

        ButtonStyle {
          background: None,
          text_color: palette.background.weakest.text,
          border: Border {
            color: Color::TRANSPARENT,
            width: 0.,
            radius: border::radius(0),
          },
          ..ButtonStyle::default()
        }
      })
      .width(100)
      .padding(0)
    ]
    .padding(SPACE_GRID)
    .width(Fill)
  ]
  .into()
}

fn login_content<'a>(model: &Model) -> Element<'a, Message> {
  column![
    text_input("Email", &model.email_input)
      .on_input(Message::UserChangedLoginInput)
      .on_submit(Message::UserSubmittedForm)
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
      .style(container::bordered_box),
    row![
      container("").width(Fill),
      button(
        text("Register")
          .align_x(text::Alignment::Right)
          .size(12)
          .color(Color::from_rgba(1., 1., 1., 0.8))
      )
      .on_press(Message::UserNavigatedRegister)
      .style(|theme: &Theme, _status| {
        let palette = theme.extended_palette();

        ButtonStyle {
          background: None,
          text_color: palette.background.weakest.text,
          border: Border {
            color: Color::TRANSPARENT,
            width: 0.,
            radius: border::radius(0),
          },
          ..ButtonStyle::default()
        }
      })
      .width(100)
      .padding(0)
    ]
    .padding(SPACE_GRID)
    .width(Fill)
  ]
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
