use crate::colors::{AccentsExt, NeutralsExt};
use crate::screens::chat::View::NoneSelected;
use crate::types::async_data::AsyncData;
use crate::{Element, SOURCE_SANS_REGULAR, chat_stream, client, icon};
use crate::{SPACE_GRID, model::Stream};
use chat_shared::convert::{IntoProto, TryIntoDomain};
use chat_shared::domain::post::{GetPostsRequest, GetPostsResponse, Post};
use chat_shared::domain::server::{Channel, ChannelType, Server, ServersResponse};
use chat_shared::domain::stream::{ClientText, ServerText, User};
use chrono::{Local, Utc};
use google_material_symbols::GoogleMaterialSymbols;
use iced::Alignment::Center;
use iced::font::Weight;
use iced::widget::keyed::column;
use iced::widget::scrollable::Scrollbar;
use iced::widget::{button, column, operation, rule, scrollable, space, text, text_input};
use iced::widget::{container, row};
use iced::{Border, Font, Length, Padding, Pixels, Task, Theme, border, padding};
use indexmap::IndexMap;
use std::str::FromStr;
use uuid::Uuid;

// --------------------------------- MODEL ---------------------------------

#[derive(Default, Clone)]
pub struct Model {
  servers: AsyncData<Vec<Server>, tonic::Status>,
  view: View,
  input: String,
}

// todo: view should model which server we're in, then which text channel we're in.
#[derive(Default, Clone)]
enum View {
  #[default]
  NoneSelected,
  TextChannel(TextChannel),
}

#[derive(Clone)]
struct TextChannel {
  id: Uuid,
  posts: AsyncData<IndexMap<Uuid, RenderedPost>, tonic::Status>,
  next_timestamp: Option<chrono::DateTime<Utc>>,
  loading_more: bool,
  name: String,
}

#[derive(Clone)]
enum RenderedPost {
  Sending {
    id: uuid::Uuid,
    created_at: chrono::DateTime<Utc>,
    content: String,
    name: String,
  },
  Errored {
    id: uuid::Uuid,
    created_at: chrono::DateTime<Utc>,
    content: String,
    name: String,
  },
  Sent(Post),
}

impl RenderedPost {
  fn created_at(&self) -> &chrono::DateTime<Utc> {
    match self {
      RenderedPost::Sending { created_at, .. }
      | RenderedPost::Errored { created_at, .. }
      | RenderedPost::Sent(Post { created_at, .. }) => created_at,
    }
  }
}

#[derive(Debug, Clone)]
pub enum Message {
  UserChangedChatInput(String),
  UserSubmittedChatInput,
  Stream(ServerText),
  UserSelectedTextChannel { text_channel_id: Uuid, name: String },
  ApiReturnedServers(Result<ServersResponse, tonic::Status>),
  ApiReturnedInitialPosts(Result<GetPostsResponse, tonic::Status>),
  ApiReturnedMorePosts(Result<GetPostsResponse, tonic::Status>),
  UserScrolledToTop,
  TypeAhead(String),
  Init,
  JoinVoice { voice_channel_id: Uuid },
  LeaveVoice,
  // Intercepted in the top-level update to (re)subscribe voice-call presence for
  // the active server. Not handled inside chat::update.
  ActiveServerChanged { server_id: Uuid },
  // Mute/deafen toggles, also intercepted by the top-level update (they need the
  // voice handle). Not handled inside chat::update.
  ToggleMute,
  ToggleDeafen,
  GoToSettings,
  None,
}

// --------------------------------- UPDATE ---------------------------------
pub fn update(
  model: &mut Model,
  message: Message,
  user: &User,
  stream: Stream<chat_stream::ChatConnection>,
) -> Task<Message> {
  match message {
    Message::UserChangedChatInput(new) => {
      model.input = new;
      Task::none()
    }
    Message::UserSubmittedChatInput => {
      let View::TextChannel(ref mut text_channel) = model.view else {
        return Task::none();
      };

      let Stream::Connected(mut stream) = stream else {
        return Task::none();
      };

      let AsyncData::Done(Ok(ref mut posts)) = text_channel.posts else {
        return Task::none();
      };

      let post_id = uuid::Uuid::new_v4();

      let send_result = stream.try_send(ClientText::CreatePostRequest {
        id: post_id,
        content: model.input.clone(),
        text_channel_id: text_channel.id,
      });

      let post = match send_result {
        Ok(_) => RenderedPost::Sending {
          id: post_id,
          created_at: chrono::Utc::now(),
          content: model.input.clone(),
          name: user.name.clone(),
        },

        Err(_) => RenderedPost::Errored {
          id: post_id,
          created_at: chrono::Utc::now(),
          content: model.input.clone(),
          name: user.name.clone(),
        },
      };

      posts.insert(post_id, post);
      model.input = "".into();

      Task::none()
    }
    Message::Stream(server_message) => match server_message {
      ServerText::JoinedRoom { from } => {
        println!("User joined room: {from:?}");
        Task::none()
      }
      ServerText::LeftRoom { from } => {
        println!("User left room: {from:?}");

        Task::none()
      }
      ServerText::Post(post) => {
        // when receiving a post, we need to check if it already exists in the posts indexmap (O(1) lookup),
        //
        // if it does, then it's a Sending and we need to reconcile - replace with Sent() and
        // reinsert based on created_at (can use bin search on the created time)
        //
        // otherwise just append post

        let View::TextChannel(ref mut text_channel) = model.view else {
          return Task::none();
        };

        let AsyncData::Done(Ok(ref mut posts)) = text_channel.posts else {
          return Task::none();
        };

        // assumption: reconciled posts will be near end of indexmap (recently sent), usually causing a small shift
        // on remove/reinsert
        posts.insert_sorted_by(post.id, RenderedPost::Sent(post), |_, v1, _, v2| {
          if v1.created_at() > v2.created_at() {
            std::cmp::Ordering::Greater
          } else {
            std::cmp::Ordering::Less
          }
        });

        Task::none()
      }
      ServerText::Pong { .. } => Task::none(),
    },
    Message::UserSelectedTextChannel {
      text_channel_id,
      name,
    } => {
      model.view = View::TextChannel(TextChannel {
        id: text_channel_id,
        posts: AsyncData::Loading,
        loading_more: false,
        next_timestamp: None,
        name,
      });
      Task::future(async move {
        let mut client = client::get().await;
        Message::ApiReturnedInitialPosts(
          client
            .posts
            .get_posts(
              GetPostsRequest {
                text_channel_id,
                limit: 50,
                starting_before_timestamp: None,
              }
              .into_proto(),
            )
            .await
            .and_then(|response| response.into_inner().try_into_domain()),
        )
      })
    }
    Message::Init => {
      model.servers = AsyncData::Loading;

      Task::future(async {
        let mut client = client::get().await;
        Message::ApiReturnedServers(
          client
            .server
            .servers(())
            .await
            .and_then(|response| response.into_inner().try_into_domain()),
        )
      })
    }
    Message::ApiReturnedServers(res) => {
      model.servers = AsyncData::Done(res.clone().map(|res| res.servers));

      // todo: for now, we load the default global server, need to pick this server in the future
      if let Ok(res) = res
        && !res.servers.is_empty()
      {
        let global_server = res.servers[0].clone();
        let server_id = global_server.id;

        let text_channel = global_server
          .channels
          .into_iter()
          .find(|channel| channel.r#type == ChannelType::Text);

        // tell the top-level update which server's call presence to watch
        let subscribe = Task::done(Message::ActiveServerChanged { server_id });

        if let Some(text_channel) = text_channel {
          Task::batch([
            subscribe,
            Task::done(Message::UserSelectedTextChannel {
              text_channel_id: text_channel.id,
              name: text_channel.name,
            }),
          ])
        } else {
          subscribe
        }
      } else {
        Task::none()
      }
    }
    Message::ApiReturnedInitialPosts(res) => {
      let View::TextChannel(ref mut text_channel) = model.view else {
        return Task::none();
      };

      if let Ok(ref res) = res {
        text_channel.next_timestamp = res.next_timestamp;
      };

      text_channel.posts = AsyncData::Done(res.map(|res| {
        IndexMap::from_iter(
          res
            .posts
            .into_iter()
            .map(|post| (post.id, RenderedPost::Sent(post))),
        )
      }));

      Task::none()
    }
    Message::None => Task::none(),
    Message::ApiReturnedMorePosts(res) => {
      let View::TextChannel(ref mut text_channel) = model.view else {
        return Task::none();
      };

      text_channel.loading_more = false; // release the prefetch guard, success or not

      let Ok(res) = res else {
        return Task::none(); // (or stash the error somewhere)
      };

      text_channel.next_timestamp = res.next_timestamp;

      if let AsyncData::Done(Ok(ref mut posts)) = text_channel.posts {
        let existing = std::mem::take(posts);

        let mut combined = IndexMap::with_capacity(existing.len() + res.posts.len());
        combined.extend(
          res
            .posts
            .into_iter()
            .map(|post| (post.id, RenderedPost::Sent(post))),
        );
        combined.extend(existing);

        *posts = combined;
      }

      Task::none()
    }
    Message::UserScrolledToTop => {
      let View::TextChannel(ref text_channel) = model.view else {
        return Task::none();
      };

      if text_channel.loading_more || text_channel.next_timestamp.is_none() {
        return Task::none();
      };

      let text_channel_id = text_channel.id;
      let starting_before_timestamp = text_channel.next_timestamp;

      Task::future(async move {
        let mut client = client::get().await;
        Message::ApiReturnedMorePosts(
          client
            .posts
            .get_posts(
              GetPostsRequest {
                text_channel_id,
                limit: 50,
                starting_before_timestamp,
              }
              .into_proto(),
            )
            .await
            .and_then(|response| response.into_inner().try_into_domain()),
        )
      })
    }
    Message::TypeAhead(input) => {
      let Some(current_text_channel_id) = (match model.view {
        View::TextChannel(TextChannel { id, .. }) => Some(id),
        _ => None,
      }) else {
        return Task::none();
      };

      let text_input_id = make_text_input_id(&current_text_channel_id);
      // model
      model.input.push_str(&input);
      Task::batch([
        operation::focus(text_input_id.clone()), // text_input::move_cursor_to_end(self.message_input_id.clone()),
        operation::move_cursor_to_end(text_input_id),
      ])
    }
    // These are all intercepted by the top-level update (they need the voice
    // handle), so they're no-ops here.
    Message::JoinVoice { .. }
    | Message::LeaveVoice
    | Message::ActiveServerChanged { .. }
    | Message::ToggleMute
    | Message::ToggleDeafen
    | Message::GoToSettings => Task::none(),
  }
}

fn make_text_input_id(text_channel_id: &Uuid) -> String {
  format!("text_input_{}", { text_channel_id.to_string() })
}

// --------------------------------- VIEW ---------------------------------

pub fn view<'a>(
  model: &'a Model,
  current_call_id: Option<&'a Uuid>,
  main_model: &'a crate::model::Model,
) -> Element<'a, Message> {
  let servers_loaded_message = match &model.servers {
    AsyncData::NotAsked | AsyncData::Loading => Some("Loading servers...".to_string()),
    AsyncData::Done(Ok(_)) => None,
    AsyncData::Done(Err(status)) => Some(format!(
      "An error occurred while loading servers: {}",
      status.code()
    )),
  };

  if let Some(loading_msg) = servers_loaded_message {
    return container(text(loading_msg).center())
      .width(Length::Fill)
      .height(Length::Fill)
      .into();
  };

  let text_chat: Element<'_, Message> = match &model.view {
    NoneSelected => container(text("No text chat selected!").center().width(Length::Fill)).into(),
    View::TextChannel(text_channel) => match &text_channel.posts {
      AsyncData::NotAsked | AsyncData::Loading => {
        // todo: Spinner here
        container(
          text("Loading posts...")
            .center()
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .into()
      }
      AsyncData::Done(Err(status)) => container(
        text(format!(
          "An error occurred while loading posts: {}",
          status.code()
        ))
        .center()
        .width(Length::Fill),
      )
      .into(),
      AsyncData::Done(Ok(posts)) => view_text_chat_window(
        &text_channel.name,
        posts,
        &model.input,
        text_channel.loading_more,
        &text_channel.id,
      ),
    },
  };

  // todo; assuming one server ever
  let channels = model
    .servers
    .as_ref()
    .map(|server| server[0].channels.as_slice())
    .get_or(&[]);

  // todo: hardcoded server name until we add server to model
  row![
    view_channels(
      "The Intergalactic Federation",
      &model.view,
      channels,
      current_call_id,
      main_model
    ),
    text_chat
  ]
  .into()
}

fn view_channels<'a>(
  server_name: &'a str,
  view: &'a View,
  channels: &'a [Channel],
  active_voice_channel_id: Option<&'a Uuid>,
  main_model: &'a crate::model::Model,
) -> Element<'a, Message> {
  let (text_channels, voice_channels): (Vec<&Channel>, Vec<&Channel>) =
    channels.iter().partition(|channel| match channel.r#type {
      ChannelType::Text => true,
      ChannelType::Voice => false,
    });

  let rendered_text: Vec<Element<'a, Message>> = text_channels
    .into_iter()
    .map(|text_channel| -> Element<'a, Message> {
      let is_selected = match view {
        NoneSelected => false,
        View::TextChannel(text_channel_view) => text_channel.id == text_channel_view.id,
      };

      button(
        row![
          icon(GoogleMaterialSymbols::Tag).size(20).style(|theme| {
            text::Style {
              color: Some(theme.extended_palette().background.weakest.text),
            }
          }),
          container(
            text(&text_channel.name)
              .wrapping(text::Wrapping::None)
              .font(Font {
                weight: Weight::Semibold,
                ..SOURCE_SANS_REGULAR
              })
          )
        ]
        .spacing((SPACE_GRID) as u32)
        .align_y(Center)
        .clip(true),
      )
      .style(move |theme: &Theme, status| -> button::Style {
        let palette = theme.extended_palette();
        let background = match status {
          button::Status::Active => {
            if is_selected {
              Some(palette.background.weak.color.into())
            } else {
              None
            }
          }
          button::Status::Hovered => Some(palette.background.weak.color.into()),
          button::Status::Pressed => Some(palette.background.weaker.color.into()),
          button::Status::Disabled => None,
        };

        button::Style {
          background,
          text_color: if is_selected || matches!(status, button::Status::Hovered) {
            palette.background.neutral.text
          } else {
            palette.background.weakest.text
          },
          border: Border {
            radius: (SPACE_GRID as u32 / 2).into(),
            ..Default::default()
          },
          ..button::Style::default()
        }
      })
      .width(Length::Fill)
      .on_press(Message::UserSelectedTextChannel {
        text_channel_id: text_channel.id,
        name: text_channel.name.clone(),
      })
      // .style(container::primary)
      .into()
    })
    .collect();

  let rendered_voice: Vec<Element<'a, Message>> = voice_channels
    .into_iter()
    .map(|voice_channel| -> Element<'a, Message> {
      let is_selected = match active_voice_channel_id {
        None => false,
        Some(id) => &voice_channel.id == id,
      };

      let selected_font_style = move |theme: &Theme| -> text::Style {
        text::Style {
          color: if is_selected {
            Some(theme.extended_palette().success.base.color)
          } else {
            None
          },
        }
      };

      let selected_icon_font_style = move |theme: &Theme| -> text::Style {
        text::Style {
          color: if is_selected {
            Some(theme.extended_palette().success.base.color)
          } else {
            None
          },
        }
      };

      button(column![
        row![
          icon(GoogleMaterialSymbols::Mic)
            .size(20)
            .style(selected_icon_font_style),
          container(
            text(&voice_channel.name)
              .wrapping(text::Wrapping::None)
              .font(Font {
                weight: Weight::Semibold,
                ..SOURCE_SANS_REGULAR
              })
              .style(selected_font_style)
          )
        ]
        .spacing((SPACE_GRID) as u32)
        .align_y(Center)
        .clip(true),
        column::Column::with_children({
          main_model
            .room_presence
            .get(&voice_channel.id)
            .map(|users| users.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|presence| -> (Uuid, Element<'a, Message>) {
              (
                presence.user.id,
                row![
                  row![
                    container(
                      text(presence.user.name.get(0..1).unwrap_or("U"))
                        .font(Font {
                          weight: Weight::Semibold,
                          ..SOURCE_SANS_REGULAR
                        })
                        .size(14)
                    )
                    .style(|theme: &Theme| {
                      container::Style {
                        text_color: Some(theme.extended_palette().background.weakest.color),
                        background: Some(theme.accents().rosewater.into()),
                        border: Border {
                          color: if !presence.speaking {
                            theme.accents().rosewater
                          } else {
                            theme.extended_palette().success.strong.color
                          },
                          width: 3.0,
                          radius: border::radius(999),
                        },
                        ..container::Style::default()
                      }
                    })
                    .center(Length::Fill)
                    .width(24)
                    .height(24),
                    text(presence.user.name.clone())
                      .font(Font {
                        weight: Weight::Semibold,
                        ..SOURCE_SANS_REGULAR
                      })
                      .size(14)
                  ]
                  .align_y(Center)
                  .spacing(SPACE_GRID as u32),
                  iced::widget::space::horizontal(),
                  row![
                    if presence.muted {
                      let el: Element<'a, Message> = icon(GoogleMaterialSymbols::MicOff).into();
                      el
                    } else {
                      let el: Element<'a, Message> = iced::widget::space().width(16).into();
                      el
                    },
                    if presence.deafened {
                      let el: Element<'a, Message> = icon(GoogleMaterialSymbols::HeadsetOff).into();
                      el
                    } else {
                      let el: Element<'a, Message> = iced::widget::space().width(16).into();
                      el
                    }
                  ]
                  .spacing(SPACE_GRID as u32)
                  .align_y(Center)
                ]
                .align_y(Center)
                .padding([0, SPACE_GRID * 3])
                .into(),
              )
            })
        })
        .spacing(SPACE_GRID as u32)
        .padding([
          if main_model
            .room_presence
            .get(&voice_channel.id)
            .is_some_and(|presence| !presence.is_empty())
          {
            SPACE_GRID / 2
          } else {
            0
          },
          0
        ]),
      ])
      .style(|theme: &Theme, status| -> button::Style {
        let palette = theme.extended_palette();
        let background = match status {
          button::Status::Active => None,
          button::Status::Hovered => Some(palette.background.stronger.color.into()),
          button::Status::Pressed => Some(palette.background.strong.color.into()),
          button::Status::Disabled => None,
        };

        button::Style {
          background,
          text_color: palette.background.base.text,
          border: Border {
            radius: (SPACE_GRID as u32 / 2).into(),
            ..Default::default()
          },
          ..button::Style::default()
        }
      })
      .width(Length::Fill)
      .on_press(Message::JoinVoice {
        voice_channel_id: voice_channel.id,
      })
      // .style(container::primary)
      .into()
    })
    .collect();

  let channel_list = scrollable(column![
    container(
      text(server_name)
        .font(Font {
          weight: Weight::Bold,
          ..SOURCE_SANS_REGULAR
        })
        .size(16)
    )
    .center(Length::Fill)
    .height((SPACE_GRID * 6) as u32 - 1)
    .width(Length::Fill),
    iced::widget::rule::horizontal(1).style(|theme: &Theme| rule::Style {
      color: theme.neutrals().crust,
      ..rule::default(theme)
    }),
    column![
      column![
        row![
          container(
            text("TEXT CHANNELS")
              .style(text::default)
              .size(12)
              .font(Font {
                weight: Weight::Semibold,
                ..SOURCE_SANS_REGULAR
              })
          )
          .padding(SPACE_GRID)
        ],
        column(rendered_text).spacing(1).width(Length::Fill),
      ],
      column![
        row![
          container(
            text("VOICE CHANNELS")
              .style(text::default)
              .size(12)
              .font(Font {
                weight: Weight::Semibold,
                ..SOURCE_SANS_REGULAR
              })
          )
          .padding(SPACE_GRID)
        ],
        column(rendered_voice).spacing(1).width(Length::Fill),
      ]
    ]
    .width(Length::Fill)
    .padding([SPACE_GRID * 2, SPACE_GRID])
    .spacing((SPACE_GRID * 2) as u32),
  ])
  .width(Length::Fill)
  .height(Length::Fill);

  let mut sidebar = column![channel_list].height(Length::Fill);

  if let Some(leave_call) = view_call_controller(active_voice_channel_id, channels, main_model) {
    sidebar = sidebar.push(leave_call);
  }

  sidebar = sidebar.push(view_user_controller(main_model));

  row![
    container(sidebar)
      .width(290)
      .style(|theme| -> container::Style {
        let palette = theme.extended_palette();

        container::Style {
          background: Some(palette.background.weakest.color.into()),
          text_color: Some(palette.background.weakest.text),
          border: border::rounded(2),
          ..container::Style::default()
        }
      })
      .height(Length::Fill),
    iced::widget::rule::vertical(1).style(|theme: &Theme| rule::Style {
      color: theme.neutrals().crust,
      ..rule::default(theme)
    })
  ]
  .into()
}

fn view_user_controller<'a>(model: &'a crate::model::Model) -> Element<'a, Message> {
  let username = match &model.user {
    crate::model::Auth::LoggedIn(user) => &user.name,
    crate::model::Auth::NotLoggedIn => "User", // no sense crashing the client; should go to
                                               // login screen
  };

  container(
    row![
      container(
        text(username.get(0..1).unwrap_or("U"))
          .font(Font {
            weight: Weight::Bold,
            ..SOURCE_SANS_REGULAR
          })
          .size(14)
      )
      .style(|theme: &Theme| {
        container::Style {
          text_color: Some(theme.neutrals().crust),
          background: Some(theme.accents().rosewater.into()),
          border: Border {
            color: theme.accents().rosewater,
            width: 3.0,
            radius: border::radius(999),
          },
          ..container::Style::default()
        }
      })
      .center(Length::Fill)
      .width(32)
      .height(32),
      column![
        text(username)
          .font(Font {
            weight: Weight::Semibold,
            ..SOURCE_SANS_REGULAR
          })
          .style(|theme: &Theme| text::Style {
            color: Some(theme.extended_palette().background.neutral.text)
          })
          .size(14)
          .line_height(1.0),
        text("Online")
          .font(Font {
            weight: Weight::Light,
            ..SOURCE_SANS_REGULAR
          })
          .size(12)
      ]
      .spacing(0),
      space::horizontal(),
      button(icon(GoogleMaterialSymbols::Settings).size(20))
        .on_press(Message::GoToSettings)
        .style(move |theme: &Theme, status| {
          let palette = theme.extended_palette();
          let text_color = palette.background.weakest.text;
          let background = match status {
            button::Status::Hovered | button::Status::Pressed => {
              Some(palette.background.weakest.color.into())
            }
            _ => None,
          };
          button::Style {
            background,
            text_color,
            border: Border {
              radius: (SPACE_GRID as u32 / 2).into(),
              ..Default::default()
            },
            ..button::Style::default()
          }
        })
    ]
    .align_y(Center)
    .spacing(SPACE_GRID as u32),
  )
  .width(Length::Fill)
  .padding(SPACE_GRID)
  .style(|theme: &Theme| {
    let neutrals = theme.neutrals();
    container::Style {
      background: Some(neutrals.mantle.into()),
      border: border::rounded(2),
      ..container::Style::default()
    }
  })
  .into()
}

// for when we display online text_chat members
// #[derive(Debug)]
// struct DisplayMember {
//   user: User,
//   status: Status,
// }
//
// #[derive(Debug)]
// enum Status {
//   Online,
//   Away,
//   DoNotDisturb,
//   Offline,
// }

// renders a self contained text chat window with input, posts and member list
fn view_text_chat_window<'a>(
  name: &'a str,
  posts: &'a IndexMap<Uuid, RenderedPost>,
  text_input_string: &'a str,
  loading_more: bool,
  text_channel_id: &'a Uuid,
) -> Element<'a, Message> {
  // let children: Element<'_, Message> = column![
  //   container(Column::with_children(posts))
  //     .padding([SPACE_GRID, 0])
  //     .height(iced::Fill),
  //   text_input("Send message", &text_input_string)
  //     .on_input(on_input)
  //     .on_submit(on_submit)
  //     .padding(SPACE_GRID)
  // ]
  // .into();

  container(column![
    // top - channel name
    view_text_chat_title(name), // middle:
    row![view_posts(posts, loading_more)].padding([0, SPACE_GRID * 2]),
    // row: text posts - user list
    container(
      text_input(&format!("Message #{name}"), text_input_string)
        .id(make_text_input_id(text_channel_id))
        .on_input(Message::UserChangedChatInput)
        .on_submit(Message::UserSubmittedChatInput)
        .style(|theme, status| {
          let default_style = text_input::default(theme, status);
          text_input::Style {
            border: Border {
              radius: (SPACE_GRID as u32).into(),
              ..default_style.border
            },
            ..default_style
          }
        })
        .padding(SPACE_GRID * 2)
    )
    .padding(Padding {
      top: 0.0,
      right: (SPACE_GRID * 2).into(),
      bottom: (SPACE_GRID * 3).into(),
      left: (SPACE_GRID * 2).into()
    }),
    // .style(|_theme: &Theme, _status| {
    //   ButtonStyle {
    //     background: Some(iced::Background::Color(Color::from_rgb(0.9, 0.9, 0.9))),
    //     text_color: Color::from_rgb(0.2, 0.2, 0.2),
    //     border: Border {
    //       radius: border::Radius::new(Pixels(SPACE_GRID.into()) / 2),
    //       ..Border::default()
    //     },
    //     ..ButtonStyle::default()
    //   }
    // }),
    // btm - input
    // Message::UserChangedChatInput,
    // Message::UserSubmittedChatInput,
  ])
  .height(Length::Fill)
  .into()
}

fn view_text_chat_title<'a>(name: &'a str) -> Element<'a, Message> {
  column![
    container(
      row![
        icon(GoogleMaterialSymbols::Tag).size(24).style(|theme| {
          text::Style {
            color: Some(theme.extended_palette().background.weakest.text),
          }
        }),
        text(name).size(18).font(Font {
          weight: Weight::Semibold,
          ..SOURCE_SANS_REGULAR
        })
      ]
      .align_y(Center)
      .padding([0, SPACE_GRID * 4])
      .spacing((SPACE_GRID) as u32)
    )
    .width(Length::Fill)
    .center_y(Length::Fill)
    .height((SPACE_GRID * 6) as u32 - 1),
    // .style(container::primary),
    iced::widget::rule::horizontal(1).style(|theme: &Theme| rule::Style {
      color: theme.neutrals().crust,
      ..rule::default(theme)
    })
  ]
  .into()
}

fn view_posts<'a>(
  posts: &'a IndexMap<Uuid, RenderedPost>,
  loading_more: bool,
) -> Element<'a, Message> {
  let mut children = vec![];
  if loading_more {
    children.push((
      uuid::Uuid::from_str("a4bbeadb-69c0-4bc6-a866-1dacde29b054").unwrap(),
      text("Loading more posts...").width(Length::Fill).into(),
    ));
  };

  let posts = posts
    .iter()
    .map(|post| {
      let (id, content, created_at, name) = match post.1 {
        RenderedPost::Sending {
          id,
          created_at,
          content,
          name,
        }
        | RenderedPost::Errored {
          id,
          created_at,
          content,
          name,
        }
        | RenderedPost::Sent(Post {
          id,
          author_name: name,
          content,
          created_at,
          ..
        }) => (id, content, created_at, name),
      };

      let display_time = created_at.with_timezone(&Local).format("%H:%M").to_string();
      let text_color = match post.1 {
        RenderedPost::Sending { .. } => text::secondary,
        RenderedPost::Errored { .. } => text::danger,
        RenderedPost::Sent(_) => text::default,
      };

      (
        *id,
        row![
          text(display_time).style(text::secondary), //.align_x(Alignment::Start),
          text(name).style(text::base),
          text(content)
            .style(text_color)
            .wrapping(text::Wrapping::WordOrGlyph)
        ]
        .spacing(Pixels(SPACE_GRID.into()))
        .into(),
      )
    })
    .collect::<Vec<(Uuid, Element<'a, Message>)>>();

  children.extend(posts);

  let scrollbar = Scrollbar::new().width(4).scroller_width(4);

  scrollable(
    column::Column::with_children(children)
      .padding(padding::right(SPACE_GRID as u32))
      .padding([SPACE_GRID, 0]),
  )
  .direction(scrollable::Direction::Vertical(scrollbar))
  .anchor_bottom()
  .on_scroll(|viewport| {
    // distance from the *top* under anchor_bottom == reversed offset
    const LOAD_THRESHOLD: f32 = 200.0; // start prefetching ~200px early
    if viewport.absolute_offset_reversed().y <= LOAD_THRESHOLD {
      Message::UserScrolledToTop
    } else {
      Message::None
    }
  })
  .height(Length::Fill)
  .width(Length::Fill)
  .into()
}

#[derive(Clone, Copy)]
enum Tone {
  Idle,
  Pending,
  Good,
  Warn,
  Bad,
}

impl Tone {
  fn color(self, theme: &Theme) -> iced::Color {
    let p = theme.extended_palette();
    match self {
      Tone::Idle => p.warning.base.color,
      Tone::Pending => p.success.weak.color,
      Tone::Good => p.success.base.color,
      Tone::Warn => p.warning.base.color,
      Tone::Bad => p.danger.base.color,
    }
  }
}

/// folds link + media into one displayed status. Media only refines `Live`.
fn describe_voice(
  link: &crate::model::LinkState,
  media: crate::model::MediaHealth,
) -> (String, Tone, Option<&'static str>) {
  use crate::model::{LinkState::*, MediaHealth};
  match link {
    Idle => ("Idle...".into(), Tone::Idle, None),
    Connecting => ("Connecting...".into(), Tone::Pending, None),
    Reconnecting { attempt } => (format!("Reconnecting... - {attempt}"), Tone::Warn, None),
    Lost { reason } => (format!("Voice Lost: {reason}"), Tone::Bad, None),
    Unstable => ("Voice Connected - Unstable".into(), Tone::Warn, None),
    Live => match media {
      MediaHealth::TransportDegraded => ("Voice Connected - Unstable".into(), Tone::Warn, None),
      MediaHealth::NoAudio => ("Voice Connected".into(), Tone::Good, None),
      MediaHealth::Flowing | MediaHealth::Unknown => ("Voice Connected".into(), Tone::Good, None),
    },
  }
}

fn latency_tone(ms: u32) -> Tone {
  match ms {
    0..=150 => Tone::Good,
    151..=300 => Tone::Warn,
    _ => Tone::Bad,
  }
}

// renders a "voice connected" footer pinned to the bottom of the sidebar.
// returns None when there's no active call, so the caller can skip pushing it.
fn view_call_controller<'a>(
  active_voice_channel_id: Option<&'a Uuid>,
  channels: &'a [Channel],
  main_model: &'a crate::model::Model,
) -> Option<Element<'a, Message>> {
  let voice_channel_id = active_voice_channel_id?;

  let channel_name = channels
    .iter()
    .find(|channel| &channel.id == voice_channel_id)
    .map(|channel| channel.name.as_str())
    .unwrap_or("Voice");

  let voice = main_model.voice.as_ref()?;
  let (status_text, tone, media_hint) = describe_voice(&voice.link_state, voice.media);

  // secondary line: channel name, then latency + media hint only while connected
  let connected = matches!(
    voice.link_state,
    crate::model::LinkState::Live | crate::model::LinkState::Unstable
  );

  let mut meta = row![
    text(channel_name)
      .size(12)
      .style(text::secondary)
      .wrapping(text::Wrapping::None)
  ]
  .spacing(6)
  .align_y(Center);

  if connected && voice.latency_ms > 0 {
    let lt = latency_tone(voice.latency_ms);
    meta = meta.push(text("·").size(12).style(text::secondary));
    meta = meta.push(
      text(format!("{} ms", voice.latency_ms))
        .size(12)
        .style(move |t: &Theme| text::Style {
          color: Some(lt.color(t)),
        }),
    );
  }

  if let Some(hint) = media_hint {
    meta = meta.push(text("·").size(12).style(text::secondary));
    meta = meta.push(text(hint).size(12).style(|t: &Theme| text::Style {
      color: Some(t.extended_palette().warning.base.color),
    }));
  }

  let status = column![
    text(status_text)
      .size(13)
      .style(move |t: &Theme| text::Style {
        color: Some(tone.color(t))
      })
      .font(Font {
        weight: Weight::Bold,
        ..SOURCE_SANS_REGULAR
      }),
    meta,
  ]
  .spacing(2)
  .width(Length::Fill);

  let mute_button = voice_toggle_button(
    if voice.muted {
      GoogleMaterialSymbols::MicOff
    } else {
      GoogleMaterialSymbols::Mic
    },
    voice.muted,
    Message::ToggleMute,
  );

  let deafen_button = voice_toggle_button(
    if voice.deafened {
      GoogleMaterialSymbols::HeadsetOff
    } else {
      GoogleMaterialSymbols::HeadsetMic
    },
    voice.deafened,
    Message::ToggleDeafen,
  );

  let leave_button = button(icon(GoogleMaterialSymbols::CallEnd).size(20))
    .on_press(Message::LeaveVoice)
    .style(|theme: &Theme, status| {
      let palette = theme.extended_palette();
      let background = match status {
        button::Status::Active => None,
        button::Status::Hovered => Some(palette.danger.weak.color.into()),
        button::Status::Pressed => Some(palette.danger.base.color.into()),
        button::Status::Disabled => None,
      };

      button::Style {
        background,
        text_color: match status {
          button::Status::Active => palette.danger.base.color,
          button::Status::Hovered => palette.background.base.text,
          button::Status::Pressed => palette.danger.base.color,
          button::Status::Disabled => palette.danger.base.color,
        },
        border: Border {
          radius: (SPACE_GRID as u32 / 2).into(),
          ..Default::default()
        },
        ..button::Style::default()
      }
    });

  let panel = container(row![status, mute_button, deafen_button, leave_button].align_y(Center))
    .width(Length::Fill)
    .padding(SPACE_GRID)
    .style(|theme: &Theme| {
      let neutrals = theme.neutrals();
      container::Style {
        background: Some(neutrals.base.into()),
        border: border::rounded(2),
        ..container::Style::default()
      }
    });

  Some(
    column![
      rule::horizontal(1).style(|theme: &Theme| rule::Style {
        color: theme.neutrals().crust,
        ..rule::default(theme)
      }),
      panel
    ]
    .into(),
  )
}

// A small icon toggle for the in-call controls (mute / deafen). When `active`
// (i.e. engaged: muted or deafened) it reads as danger; otherwise it's subtle.
fn voice_toggle_button<'a>(
  symbol: GoogleMaterialSymbols,
  active: bool,
  on_press: Message,
) -> Element<'a, Message> {
  button(icon(symbol).size(20))
    .on_press(on_press)
    .style(move |theme: &Theme, status| {
      let palette = theme.extended_palette();
      let text_color = if active {
        palette.warning.base.color
      } else {
        palette.background.neutral.text
      };
      let background = match status {
        button::Status::Hovered | button::Status::Pressed => {
          Some(palette.background.weakest.color.into())
        }
        _ => None,
      };
      button::Style {
        background,
        text_color,
        border: Border {
          radius: (SPACE_GRID as u32 / 2).into(),
          ..Default::default()
        },
        ..button::Style::default()
      }
    })
    .into()
}
