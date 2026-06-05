use chat_rs::shared::{
  convert::{IntoProto, TryIntoDomain},
  domain::auth::{
    LoginCommand, LoginReturn, RegisterCommand, RegisterReturn, Token, VerifyCommand, VerifyReturn,
  },
};
use email_address;
use std::{str::FromStr, sync::Arc};

use iced::{
  Border, Color, Element,
  Length::{Fill, FillPortion},
  Pixels, Task, Theme,
  border::{self},
  widget::{
    button, button::Style as ButtonStyle, column, container, container::Style as ContainerStyle,
    float, row, space, text, text_input, text_input::Style as TextStyle,
  },
};

use crate::{
  SPACE_GRID,
  client::{self},
};

// -------------------- MODEL --------------------

#[derive(Debug)]
pub enum Mode {
  Login {
    error_message: Option<&'static str>,
  },
  Register {
    error_message: Option<&'static str>,
    username_input: String,
  },
  Code {
    error_message: Option<&'static str>,
    identifier: Token,
    code_input: String,
  },
}

impl Default for Mode {
  fn default() -> Self {
    Self::Login {
      error_message: None,
    }
  }
}

#[derive(Debug)]
pub struct Model {
  mode: Mode,
  email_input: String,
  remember_me_checked: bool,
}

impl Model {
  pub fn new() -> Self {
    Model {
      mode: Default::default(),
      email_input: Default::default(),
      remember_me_checked: Default::default(),
    }
  }

  fn action_text_for_mode(&self) -> &str {
    match &self.mode {
      Mode::Code { .. } => "Submit",
      Mode::Login { .. } => "Login",
      Mode::Register { .. } => "Register",
    }
  }
}

#[derive(Debug, Clone)]
pub enum Message {
  UserChangedEmailInput(String),
  UserChangedUsernameInput(String),
  UserChangedCodeInput(String),

  UserSubmittedForm,
  UserNavigatedRegister,
  UserNavigatedLogin,
  UserToggledRememberMe,
  ApiSentLogin(Result<LoginReturn, Arc<tonic::Status>>),
  ApiSentRegister(Result<RegisterReturn, Arc<tonic::Status>>),
  ApiVerifiedCode(Result<VerifyReturn, Arc<tonic::Status>>),
  // None,
}

// -------------------- UPDATE --------------------

pub fn update(model: &mut Model, message: Message) -> Task<Message> {
  match message {
    Message::UserChangedEmailInput(new) => {
      if let Mode::Login { .. } | Mode::Register { .. } = model.mode {
        model.email_input = new;
      };

      Task::none()
    }
    Message::UserChangedCodeInput(new) => {
      if let Mode::Code {
        ref mut code_input, ..
      } = model.mode
      {
        *code_input = new;
      };

      Task::none()
    }
    Message::UserChangedUsernameInput(new) => {
      if let Mode::Register {
        ref mut username_input,
        ..
      } = model.mode
      {
        *username_input = new;
      };

      Task::none()
    }
    Message::UserSubmittedForm => match &mut model.mode {
      Mode::Login { error_message } => {
        if let Err(_valid) = email_address::EmailAddress::from_str(&model.email_input) {
          *error_message = Some("Invalid email.");
          Task::none()
        } else {
          let email_input = model.email_input.clone();
          Task::perform(
            async move {
              client::get()
                .await
                .auth
                .login(LoginCommand { email: email_input }.into_proto())
                .await
            },
            move |response| {
              Message::ApiSentLogin(
                response
                  .map(|res| res.into_inner().try_into_domain().unwrap())
                  .map_err(Arc::new),
              )
            },
          )
        }
      }
      Mode::Register {
        error_message,
        username_input,
      } => {
        if let Err(_valid) = email_address::EmailAddress::from_str(&model.email_input) {
          *error_message = Some("Invalid email.");
          Task::none()
        } else {
          let email_input = model.email_input.clone();
          let username_input = username_input.clone();

          Task::perform(
            async move {
              client::get()
                .await
                .auth
                .register(
                  RegisterCommand {
                    email: email_input,
                    username: username_input,
                  }
                  .into_proto(),
                )
                .await
            },
            move |response| {
              Message::ApiSentRegister(
                response
                  .map(|res| res.into_inner().try_into_domain().unwrap())
                  .map_err(Arc::new),
              )
            },
          )
        }
      }
      Mode::Code {
        error_message: _,
        identifier,
        code_input,
      } => {
        let email_input = model.email_input.clone();
        let code_input = code_input.clone();
        let identifier = identifier.clone();

        Task::perform(
          async move {
            client::get()
              .await
              .auth
              .verify(
                VerifyCommand {
                  identifier: identifier.try_into().unwrap(),
                  email: email_input,
                  code: code_input,
                }
                .into_proto(),
              )
              .await
          },
          |response| {
            Message::ApiVerifiedCode(
              response
                .map(|res| res.into_inner().try_into_domain().unwrap())
                .map_err(Arc::new),
            )
          },
        )
      }
    },
    Message::UserToggledRememberMe => {
      model.remember_me_checked = !model.remember_me_checked;
      Task::none()
    }
    Message::UserNavigatedRegister => {
      model.mode = Mode::Register {
        error_message: None,
        username_input: "".to_string(),
      };
      Task::none()
    }
    Message::UserNavigatedLogin => {
      model.mode = Mode::Login {
        error_message: None,
      };
      Task::none()
    }
    Message::ApiSentLogin(response) => {
      let new_err_msg = response
        .clone()
        .map_err(|err| match err.code() {
          tonic::Code::NotFound => "Email was not found.",
          _ => "An error ocurred while sending your confirmation email. Please try again later.",
        })
        .map(|_| -> Option<&str> { None })
        .err();

      match (&mut model.mode, response) {
        (Mode::Login { error_message }, Err(_)) => {
          *error_message = new_err_msg;
        }
        (Mode::Login { .. }, Ok(res)) => {
          model.mode = Mode::Code {
            error_message: None,
            identifier: res.identifier.into(),
            code_input: "".to_string(),
          }
        }
        (_, _) => (),
      };

      Task::none()
    }
    Message::ApiVerifiedCode(Ok(body)) => Task::future(async {
      client::get().await.insert_tokens(body).await;
      Message::UserNavigatedLogin
    }),
    Message::ApiVerifiedCode(Err(status)) => {
      // VerifyError::InvalidCode => Status::unauthenticated("invalid code"),
      // VerifyError::UnknownIdentifier => Status::not_found("unknown identifier"),
      // VerifyError::Internal => Status::internal("internal error"),
      match status.code() {
        tonic::Code::Unauthenticated => {
          if let Mode::Code {
            ref mut error_message,
            ..
          } = model.mode
          {
            *error_message = Some("Invalid code. Please double check your code and try again.");
          }
        }
        tonic::Code::NotFound => {
          model.mode = Mode::Login {
            error_message: Some("Your code has expired. Please request a new one."),
          };
        }
        _ => {
          if let Mode::Code {
            ref mut error_message,
            ..
          } = model.mode
          {
            *error_message = Some("An unknown error occurred.");
          }
        }
      };

      Task::none()
    }
    // Message::None => Task::none(),
    Message::ApiSentRegister(response) => {
      let new_err_msg = response
        .clone()
        .map_err(|err| match err.code() {
          tonic::Code::AlreadyExists => "Email already exists, please login instead.",
          _ => "An error ocurred while sending your confirmation email. Please try again later.",
        })
        .map(|_| -> Option<&str> { None })
        .err();

      match (&mut model.mode, response) {
        (Mode::Register { error_message, .. }, Err(_)) => {
          *error_message = new_err_msg;
        }
        (Mode::Register { .. }, Ok(res)) => {
          model.mode = Mode::Code {
            error_message: None,
            identifier: res.identifier.into(),
            code_input: "".to_string(),
          }
        }
        (_, _) => (), // ignore responses when already in code view
      };

      Task::none()
    }
  }
}

// bridge: add loading states, use async data
// -------------------- VIEW --------------------

pub fn view<'a>(model: &'a Model) -> Element<'a, Message> {
  row![left_card(model), hero()]
    .spacing(Pixels(SPACE_GRID.into()))
    .width(Fill)
    .height(Fill)
    .into()
}

fn left_card<'a>(model: &'a Model) -> Element<'a, Message> {
  container(
    container(
      column![
        render_left_content(model),
        button(container(model.action_text_for_mode()).center_x(Fill))
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
          match model.mode {
            Mode::Login { error_message }
            | Mode::Register { error_message, .. }
            | Mode::Code { error_message, .. } => {
              if let Some(err) = error_message {
                float(text(err).color(Color::from_rgb(1., 0.5, 0.5)).size(12))
              } else {
                float(text("").height(0))
              }
            }
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
  match &model.mode {
    Mode::Login { .. } => login_content(model),
    Mode::Register { username_input, .. } => register_content(model, username_input),
    Mode::Code { code_input, .. } => code_input_content(model, code_input),
  }
}

fn register_content<'a>(model: &Model, username_input: &str) -> Element<'a, Message> {
  column![
    text_input("Email", &model.email_input)
      .on_input(Message::UserChangedEmailInput)
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
    space().height(8),
    text_input("Username", username_input)
      .on_input(Message::UserChangedUsernameInput)
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
        text("Login")
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

fn code_input_content<'a>(_model: &Model, code_input: &str) -> Element<'a, Message> {
  column![
    text_input("Enter Code", code_input)
      .on_input(Message::UserChangedCodeInput)
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
      .on_input(Message::UserChangedEmailInput)
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
