use crate::colors::{AccentsExt, NeutralsExt};
use crate::screens::chat::View::NoneSelected;
use crate::types::async_data::AsyncData;
use crate::widgets::context_menu::ContextMenu;
use crate::{Element, SOURCE_SANS_REGULAR, chat_stream, client, icon};
use crate::{SPACE_GRID, model::Stream};
use chat_shared::convert::{IntoProto, TryIntoDomain};
use chat_shared::domain::post::{GetPostsRequest, GetPostsResponse, Post};
use chat_shared::domain::server::{Channel, ChannelType, Server, ServersResponse};
use chat_shared::domain::stream::{ClientText, ServerText, User};
use chrono::{Datelike, Local, Utc};
use google_material_symbols::GoogleMaterialSymbols;
use iced::Alignment::Center;
use iced::font::Weight;
use iced::widget::keyed::column;
use iced::widget::scrollable::Scrollbar;
use iced::widget::{
  button, column, operation, rule, scrollable, slider, space, stack, text, text_editor,
};
use iced::widget::{container, row};
use iced::{Border, Color, Font, Length, Padding, Pixels, Task, Theme, border, padding};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, Instant};

/// How often, at most, we announce our own typing to the server while editing.
const TYPING_SEND_THROTTLE: Duration = Duration::from_millis(2500);
/// How long a received typing indicator lingers before it's expired locally.
/// Must exceed `TYPING_SEND_THROTTLE` so a steadily-typing peer never flickers.
const TYPING_TIMEOUT: Duration = Duration::from_secs(5);
use uuid::Uuid;

// --------------------------------- MODEL ---------------------------------

#[derive(Default)]
pub struct Model {
  servers: AsyncData<Vec<Server>, tonic::Status>,
  view: View,
  // multi-line editor buffer (Shift+Enter inserts a newline; Enter submits).
  input: text_editor::Content,
  // peers currently typing, keyed channel -> (user id -> entry). The IndexMap
  // keeps a stable "who started first" order for the indicator. Entries expire
  // via delayed ExpireTyping tasks; `seq` lets a stale expiry skip a peer who
  // has typed again since it was scheduled.
  typing: HashMap<Uuid, IndexMap<Uuid, TypingPeer>>,
  typing_seq: u64,
  // when we last announced our own typing, so editing doesn't spam the server.
  last_typing_sent: Option<Instant>,
}

#[derive(Clone)]
struct TypingPeer {
  name: String,
  seq: u64,
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

  fn author_name(&self) -> &str {
    match self {
      RenderedPost::Sending { name, .. } | RenderedPost::Errored { name, .. } => name,
      RenderedPost::Sent(Post { author_name, .. }) => author_name,
    }
  }
}

#[derive(Debug, Clone)]
pub enum Message {
  EditorAction(text_editor::Action),
  UserSubmittedChatInput,
  // a received typing indicator timed out; clear it if not refreshed since.
  ExpireTyping {
    text_channel_id: Uuid,
    user_id: Uuid,
    seq: u64,
  },
  Stream(ServerText),
  UserSelectedTextChannel {
    text_channel_id: Uuid,
    name: String,
  },
  ApiReturnedServers(Result<ServersResponse, tonic::Status>),
  ApiReturnedInitialPosts(Result<GetPostsResponse, tonic::Status>),
  ApiReturnedMorePosts(Result<GetPostsResponse, tonic::Status>),
  UserScrolledToTop,
  TypeAhead(String),
  Init,
  JoinVoice {
    voice_channel_id: Uuid,
  },
  LeaveVoice,
  // Intercepted in the top-level update to (re)subscribe voice-call presence for
  // the active server. Not handled inside chat::update.
  ActiveServerChanged {
    server_id: Uuid,
  },
  // Mute/deafen toggles, also intercepted by the top-level update (they need the
  // voice handle). Not handled inside chat::update.
  ToggleMute,
  ToggleDeafen,
  // Per-user mixer, set from a participant's right-click menu. Both are
  // intercepted by the top-level update (they touch persisted prefs + the voice
  // handle). `volume` is a linear 0.0..=2.0 multiplier. Not handled in chat::update.
  SetUserVolume {
    user_id: Uuid,
    volume: f32,
  },
  // Slider released: persist the level once, instead of writing the file on
  // every drag step (mirrors the noise-gate slider in the settings screen).
  UserVolumeReleased {
    user_id: Uuid,
  },
  ToggleUserMute {
    user_id: Uuid,
  },
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
    Message::EditorAction(action) => {
      let is_edit = action.is_edit();
      model.input.perform(action);

      // While the user is actively editing in a channel, announce typing to the
      // server — throttled, and never for an empty buffer (e.g. just cleared).
      let channel_id = match model.view {
        View::TextChannel(TextChannel { id, .. }) => Some(id),
        _ => None,
      };
      if is_edit
        && let Some(channel_id) = channel_id
        && let Stream::Connected(mut stream) = stream
      {
        let now = Instant::now();
        let due = model
          .last_typing_sent
          .is_none_or(|last| now.duration_since(last) >= TYPING_SEND_THROTTLE);
        if due && !model.input.text().trim().is_empty() {
          model.last_typing_sent = Some(now);
          let _ = stream.try_send(ClientText::Typing {
            text_channel_id: channel_id,
          });
        }
      }
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

      let content = model.input.text();
      // ignore submits with no real content (blank lines / whitespace only).
      if content.trim().is_empty() {
        return Task::none();
      }

      let post_id = uuid::Uuid::new_v4();

      let send_result = stream.try_send(ClientText::CreatePostRequest {
        id: post_id,
        content: content.clone(),
        text_channel_id: text_channel.id,
      });

      let post = match send_result {
        Ok(_) => RenderedPost::Sending {
          id: post_id,
          created_at: chrono::Utc::now(),
          content: content.clone(),
          name: user.name.clone(),
        },

        Err(_) => RenderedPost::Errored {
          id: post_id,
          created_at: chrono::Utc::now(),
          content,
          name: user.name.clone(),
        },
      };

      posts.insert(post_id, post);
      model.input = text_editor::Content::new();
      // sending implies we're no longer typing; let the next edit re-announce.
      model.last_typing_sent = None;

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
        let channel_id = text_channel.id;
        // a delivered post means its author stopped typing. Post carries no user
        // id, so clear by author name (best-effort; the timeout covers the rest).
        let author_name = post.author_name.clone();

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

        if let Some(channel_typing) = model.typing.get_mut(&channel_id) {
          channel_typing.retain(|_, peer| peer.name != author_name);
        }
        if model.typing.get(&channel_id).is_some_and(|c| c.is_empty()) {
          model.typing.remove(&channel_id);
        }

        Task::none()
      }
      ServerText::Pong { .. } => Task::none(),
      ServerText::Typing {
        from,
        text_channel_id,
      } => {
        model.typing_seq += 1;
        let seq = model.typing_seq;
        let user_id = from.id;
        model.typing.entry(text_channel_id).or_default().insert(
          user_id,
          TypingPeer {
            name: from.name,
            seq,
          },
        );

        // schedule this entry's expiry; a newer event bumps `seq` so this one
        // becomes a no-op (see Message::ExpireTyping).
        Task::future(async move {
          tokio::time::sleep(TYPING_TIMEOUT).await;
          Message::ExpireTyping {
            text_channel_id,
            user_id,
            seq,
          }
        })
      }
    },
    Message::ExpireTyping {
      text_channel_id,
      user_id,
      seq,
    } => {
      if let Some(channel_typing) = model.typing.get_mut(&text_channel_id) {
        // only remove if this peer hasn't typed again since we scheduled this.
        if channel_typing
          .get(&user_id)
          .is_some_and(|peer| peer.seq == seq)
        {
          channel_typing.shift_remove(&user_id);
        }
      }
      if model
        .typing
        .get(&text_channel_id)
        .is_some_and(|c| c.is_empty())
      {
        model.typing.remove(&text_channel_id);
      }
      Task::none()
    }
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
      // insert the typed-ahead text at the cursor (Content has no push_str), then
      // focus the editor so subsequent keystrokes land there.
      for c in input.chars() {
        model
          .input
          .perform(text_editor::Action::Edit(text_editor::Edit::Insert(c)));
      }
      operation::focus(text_input_id)
    }
    // These are all intercepted by the top-level update (they need the voice
    // handle), so they're no-ops here.
    Message::JoinVoice { .. }
    | Message::LeaveVoice
    | Message::ActiveServerChanged { .. }
    | Message::ToggleMute
    | Message::ToggleDeafen
    | Message::SetUserVolume { .. }
    | Message::UserVolumeReleased { .. }
    | Message::ToggleUserMute { .. }
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
      AsyncData::Done(Ok(posts)) => {
        let typing_names: Vec<&str> = model
          .typing
          .get(&text_channel.id)
          .map(|peers| peers.values().map(|p| p.name.as_str()).collect())
          .unwrap_or_default();
        view_text_chat_window(
          &text_channel.name,
          posts,
          &model.input,
          text_channel.loading_more,
          &text_channel.id,
          typing_names,
        )
      }
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

      let header_row = row![
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
      .clip(true);

      // The header is the join affordance — a button only while we're NOT in this
      // call. Once joined it becomes a plain row, so clicking it can't trigger a
      // redundant rejoin, and only the header (not the whole call card with its
      // participant list) reacts to hover.
      // Always a button so the header keeps identical padding/layout whether or
      // not we're in the call. When joined we just omit `on_press`, which makes
      // the button inert — it can't trigger a redundant rejoin and shows no hover
      // highlight (its status stays Disabled).
      let header_button = button(header_row)
        .style(|theme: &Theme, status| -> button::Style {
          let palette = theme.extended_palette();
          let background = match status {
            button::Status::Hovered => Some(palette.background.stronger.color.into()),
            button::Status::Pressed => Some(palette.background.strong.color.into()),
            // Active + Disabled (joined): no highlight.
            _ => None,
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
        .width(Length::Fill);

      let header: Element<'a, Message> = if is_selected {
        header_button.into()
      } else {
        header_button
          .on_press(Message::JoinVoice {
            voice_channel_id: voice_channel.id,
          })
          .into()
      };

      // NON-keyed column on purpose: rows are heterogeneous (our own row is a
      // plain container, peers are stateful ContextMenus). A keyed column
      // reconciles reordered children with a non-tag-checking diff, which can
      // pair a ContextMenu element with a leftover stateless tree when the
      // roster changes — and then panics ("Downcast on stateless state") on the
      // overlay pass. A plain Column tag-checks each child and rebuilds state on
      // a type mismatch, so it stays correct as people join/leave.
      let participants = iced::widget::Column::with_children({
        main_model
          .room_presence
          .get(&voice_channel.id)
          .map(|users| users.as_slice())
          .unwrap_or(&[])
          .iter()
          .map(|presence| -> Element<'a, Message> {
            let muted_by_me = main_model
              .per_user_audio
              .get(&presence.user.id)
              .is_some_and(|p| p.muted);
            let speaking_ring = presence.speaking && !muted_by_me;
            let row_el: Element<'a, Message> = row![
              row![
                container(
                  text(presence.user.name.get(0..1).unwrap_or("U").to_uppercase())
                    .font(Font {
                      weight: Weight::Semibold,
                      ..SOURCE_SANS_REGULAR
                    })
                    .size(14)
                )
                .style(move |theme: &Theme| {
                  container::Style {
                    text_color: Some(theme.extended_palette().background.weakest.color),
                    background: Some(theme.accents().rosewater.into()),
                    border: Border {
                      color: if !speaking_ring {
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
                if muted_by_me {
                  let el: Element<'a, Message> = icon(GoogleMaterialSymbols::MicOff)
                    .style(|theme: &Theme| iced::widget::text::Style {
                      color: Some(theme.accents().red),
                    })
                    .into();
                  el
                } else {
                  let el: Element<'a, Message> = iced::widget::space().width(16).into();
                  el
                },
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
            .into();

            // Right-click a *remote* participant to open their personal volume
            // mixer (no menu on our own row — there's nothing to adjust). The
            // current level comes from the persisted per-user prefs, defaulting
            // to unity when we've never touched this user.
            let me_id = match &main_model.user {
              crate::model::Auth::LoggedIn(u) => Some(u.id),
              _ => None,
            };
            let user_id = presence.user.id;
            let element: Element<'a, Message> = if Some(user_id) == me_id {
              // our own row: nothing to adjust. Plain container, padded to line
              // up with the interactive rows below.
              container(row_el)
                .padding(SPACE_GRID / 4)
                .width(Length::Fill)
                .into()
            } else {
              let pref = main_model
                .per_user_audio
                .get(&user_id)
                .copied()
                .unwrap_or_default();
              let name = presence.user.name.clone();
              // wrap each peer in a button purely for the per-row hover
              // highlight, signalling "this row is interactive" so the
              // right-click mixer is discoverable. Left-press is a no-op; the
              // mixer opens on right-click via the ContextMenu.
              let hover_row = button(row_el)
                .padding(SPACE_GRID / 4)
                .width(Length::Fill)
                .on_press(Message::None)
                .style(|theme: &Theme, status| -> button::Style {
                  let palette = theme.extended_palette();
                  button::Style {
                    background: match status {
                      button::Status::Hovered | button::Status::Pressed => {
                        Some(palette.background.weak.color.into())
                      }
                      _ => None,
                    },
                    text_color: palette.background.base.text,
                    border: Border {
                      radius: (SPACE_GRID as u32 / 2).into(),
                      ..Default::default()
                    },
                    ..button::Style::default()
                  }
                });
              ContextMenu::new(hover_row, move || {
                user_mixer_overlay(user_id, name.clone(), pref)
              })
              .into()
            };
            element
          })
      })
      .spacing((SPACE_GRID as u32) / 2)
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
        0,
      ]);

      column![header, participants].width(Length::Fill).into()
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

  // mute/deafen persist across calls, so they live in the always-present user
  // controller (Discord-style) rather than only inside the active-call panel.
  let (muted, deafened) = model
    .voice
    .as_ref()
    .map(|voice| (voice.muted, voice.deafened))
    .unwrap_or((false, false));

  let mute_button = voice_toggle_button(
    if muted {
      GoogleMaterialSymbols::MicOff
    } else {
      GoogleMaterialSymbols::Mic
    },
    muted,
    Message::ToggleMute,
  );

  let deafen_button = voice_toggle_button(
    if deafened {
      GoogleMaterialSymbols::HeadsetOff
    } else {
      GoogleMaterialSymbols::HeadsetMic
    },
    deafened,
    Message::ToggleDeafen,
  );

  container(
    row![
      container(
        text(username.get(0..1).unwrap_or("U").to_uppercase())
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
      mute_button,
      deafen_button,
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
  input: &'a text_editor::Content,
  loading_more: bool,
  text_channel_id: &'a Uuid,
  typing_names: Vec<&'a str>,
) -> Element<'a, Message> {
  let editor = text_editor(input)
    .placeholder(format!("Message #{name}"))
    .id(make_text_input_id(text_channel_id))
    .on_action(Message::EditorAction)
    // Enter submits; Shift+Enter inserts a newline (the editor's default Enter).
    // Custom skips the buffer edit, so the submit newline never lands in `input`.
    .key_binding(|key_press| {
      use iced::keyboard::Key;
      use iced::keyboard::key::Named;
      if matches!(&key_press.key, Key::Named(Named::Enter)) && !key_press.modifiers.shift() {
        Some(text_editor::Binding::Custom(
          Message::UserSubmittedChatInput,
        ))
      } else {
        text_editor::Binding::from_key_press(key_press)
      }
    })
    // grows with content (Shrink height) up to a cap, then scrolls internally —
    // so a long multi-line draft can't push the message list off-screen.
    .max_height((SPACE_GRID * 24) as f32)
    .padding(SPACE_GRID * 2)
    .style(|theme, status| {
      let default_style = text_editor::default(theme, status);
      text_editor::Style {
        border: Border {
          radius: (SPACE_GRID as u32).into(),
          ..default_style.border
        },
        ..default_style
      }
    });

  container(column![
    // top - channel name
    view_text_chat_title(name),
    // The message list fills the space above the input. The Discord-style typing
    // line is floated transparently over the *bottom* of this list (just above
    // the input) rather than taking its own row — so it reserves no gap and the
    // messages scroll behind it. It's non-interactive, so scrolling the list
    // beneath is unaffected.
    stack![
      row![view_posts(posts, loading_more)].padding([0, SPACE_GRID * 2]),
      container(view_typing_indicator(&typing_names))
        .width(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(Padding {
          top: 0.0,
          right: (SPACE_GRID * 2).into(),
          bottom: 0.0,
          left: (SPACE_GRID * 2).into(),
        }),
    ],
    // bottom - the message input. Fill width so the editor is bounded and wraps
    // long lines (a Shrink container would give it unbounded width = no wrap).
    container(editor).width(Length::Fill).padding(Padding {
      top: 0.0,
      right: (SPACE_GRID * 2).into(),
      bottom: (SPACE_GRID * 3).into(),
      left: (SPACE_GRID * 2).into(),
    }),
  ])
  .height(Length::Fill)
  .into()
}

/// The "X is typing…" line above the input. Returns a fixed-height element even
/// when nobody's typing, so the input doesn't jump as it appears/disappears.
fn view_typing_indicator<'a>(names: &[&str]) -> Element<'a, Message> {
  let line = match names {
    [] => None,
    [a] => Some(format!("{a} is typing…")),
    [a, b] => Some(format!("{a} and {b} are typing…")),
    [a, b, c] => Some(format!("{a}, {b}, and {c} are typing…")),
    _ => Some("Several people are typing…".to_string()),
  };

  match line {
    // Active: an opaque "pill" behind the text so it reads cleanly instead of
    // clipping with the messages scrolling behind it. Background matches the
    // chat area (root `base`), so it occludes without looking like a foreign box.
    Some(line) => container(text(line).size(12).style(|theme: &Theme| text::Style {
      color: Some(theme.extended_palette().background.weak.text),
    }))
    .width(Length::Fill)
    .padding([SPACE_GRID / 4, SPACE_GRID / 2])
    .style(|theme: &Theme| container::Style {
      background: Some(theme.extended_palette().background.weak.color.into()),
      border: border::rounded((SPACE_GRID / 2) as f32),
      ..container::Style::default()
    })
    .into(),
    // Inactive: render nothing (the overlay reserves no space of its own).
    None => space().into(),
  }
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

fn view_day_divider<'a>(date: chrono::NaiveDate) -> (Uuid, Element<'a, Message>) {
  let today = Local::now().date_naive();
  let label = match (today - date).num_days() {
    0 => "Today".to_string(),
    1 => "Yesterday".to_string(),
    _ => date.format("%A, %B %-d, %Y").to_string(),
  };

  // deterministic, stable key per calendar day so keyed diffing stays consistent
  let key = Uuid::from_u64_pair(0xda7e_da7e_da7e_da7e, date.num_days_from_ce() as u64);

  let line = || {
    rule::horizontal(1).style(|theme: &Theme| rule::Style {
      color: theme.extended_palette().secondary.weak.color,
      ..rule::default(theme)
    })
  };

  (
    key,
    row![
      container(line()).width(Length::Fill).center_y(Length::Fill),
      text(label).size(12).style(|theme| text::Style {
        color: text::secondary(theme).color,
      }),
      container(line()).width(Length::Fill).center_y(Length::Fill),
    ]
    .align_y(Center)
    .spacing(SPACE_GRID as u32)
    .padding([SPACE_GRID, 0])
    .into(),
  )
}

// The author-name column is sized to the widest name present in the channel so
// message content always starts at the same horizontal offset, just past the
// longest name. Names are truncated to `NAME_MAX_CHARS` first, so one very long
// name can't push the whole column across the screen.
const NAME_MAX_CHARS: usize = 22;

fn truncate_name(name: &str) -> String {
  if name.chars().count() <= NAME_MAX_CHARS {
    name.to_string()
  } else {
    let truncated: String = name.chars().take(NAME_MAX_CHARS - 1).collect();
    format!("{truncated}…")
  }
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

  // Widest (truncated) name in the channel — drives the name-column width below.
  let longest_name = posts
    .values()
    .map(|post| truncate_name(post.author_name()))
    .max_by_key(|name| name.chars().count())
    .unwrap_or_default();

  let mut previous_date: Option<chrono::NaiveDate> = None;
  let mut previous_author: Option<&str> = None;
  for post in posts.iter() {
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

    let local = created_at.with_timezone(&Local);
    let date = local.date_naive();
    let is_new_day = previous_date != Some(date);
    if is_new_day {
      children.push(view_day_divider(date));
      previous_date = Some(date);
    }

    // Hide the name when this message continues a run from the same author (but
    // always show it after a day divider). The sizer below keeps content aligned.
    let display_name = if !is_new_day && previous_author == Some(name.as_str()) {
      String::new()
    } else {
      truncate_name(name)
    };
    previous_author = Some(name);

    let display_time = local.format("%H:%M").to_string();
    let text_color = match post.1 {
      RenderedPost::Sending { .. } => text::secondary,
      RenderedPost::Errored { .. } => text::danger,
      RenderedPost::Sent(_) => text::default,
    };

    children.push((
      *id,
      row![
        iced_selection::text(display_time).style(|theme| iced_selection::text::Style {
          color: text::secondary(theme).color,
          selection: theme.extended_palette().secondary.strong.text
        }), //.align_x(Alignment::Start),
        stack![
          // invisible sizer reserving the width of the widest name in the channel,
          // plus a little left padding so right-aligned names don't clip
          container(
            text(longest_name.clone())
              .wrapping(text::Wrapping::None)
              .style(|_theme| text::Style {
                color: Some(Color::TRANSPARENT)
              })
          )
          .padding(padding::left(SPACE_GRID as f32 * 2.0)),
          iced_selection::text(display_name)
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right)
            .wrapping(text::Wrapping::None)
            .style(|theme| iced_selection::text::Style {
              color: text::base(theme).color,
              selection: theme.extended_palette().secondary.strong.text
            }),
        ],
        iced_selection::text(content)
          .style(move |theme| iced_selection::text::Style {
            color: text_color(theme).color,
            selection: theme.extended_palette().secondary.strong.text
          })
          .wrapping(text::Wrapping::WordOrGlyph)
      ]
      .spacing(Pixels(SPACE_GRID.into()))
      .into(),
    ));
  }

  let scrollbar = Scrollbar::new().width(4).scroller_width(4);

  scrollable(
    column::Column::with_children(children).padding(padding::Padding {
      top: SPACE_GRID as f32,
      right: SPACE_GRID as f32 * 2.0,
      // extra bottom whitespace so the floating typing indicator doesn't occlude
      // the latest message/timestamp when scrolled to the bottom.
      bottom: SPACE_GRID as f32 * 3.0,
      left: 0.0,
    }),
  )
  .direction(scrollable::Direction::Vertical(scrollbar))
  .anchor_bottom()
  .on_scroll(|viewport| {
    // distance from the *top* under anchor_bottom == reversed offset
    const LOAD_THRESHOLD: f32 = 400.0; // start prefetching ~200px early
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
/// `input_ok`/`output_ok` flag a dead mic/speaker: the user joined, but that
/// direction is silent until they fix it. A dead device wins over the normal
/// "Voice Connected" line because it's the most actionable thing on screen.
fn describe_voice(
  link: &crate::model::LinkState,
  media: chat_core::voice::MediaHealth,
  input_ok: bool,
  output_ok: bool,
  mic_receiving: bool,
) -> (String, Tone, Option<String>) {
  use chat_core::voice::MediaHealth;

  use crate::model::LinkState::*;

  // a dead device is only meaningful once we're actually in the call. The mic
  // counts as down either when its device failed to open (`!input_ok`) or when
  // it opened but no frames are arriving (`!mic_receiving`) — unplugged, muted
  // at the OS, or broken. Same warning for both: the user knows the context.
  let connected = matches!(link, Live | Unstable);
  let mic_down = !input_ok || !mic_receiving;
  let speaker_down = !output_ok;
  let device_hint: Option<String> = match (connected, mic_down, speaker_down) {
    (true, true, true) => Some("No mic or speaker — check Settings".into()),
    (true, true, false) => Some("No mic — others can't hear you".into()),
    (true, false, true) => Some("No speaker — you can't hear others".into()),
    _ => None,
  };

  let (status, tone, media_hint): (String, Tone, Option<String>) = match link {
    Idle => ("Idle...".into(), Tone::Idle, None),
    Connecting => ("Connecting...".into(), Tone::Pending, None),
    Reconnecting { attempt } => (format!("Reconnecting... - {attempt}"), Tone::Warn, None),
    Lost { reason } => (format!("Voice Lost: {reason}"), Tone::Bad, None),
    Unstable => ("Voice Connected - Unstable".into(), Tone::Warn, None),
    Live => match media {
      MediaHealth::TransportDegraded => ("Voice Connected - Unstable".into(), Tone::Warn, None),
      MediaHealth::NoAudio | MediaHealth::Flowing | MediaHealth::Unknown => {
        ("Voice Connected".into(), Tone::Good, None)
      }
    },
  };

  // a dead device downgrades the tone and replaces the hint with the fix-it text.
  match device_hint {
    Some(hint) => (status, Tone::Warn, Some(hint)),
    None => (status, tone, media_hint),
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
  let (status_text, tone, media_hint) = describe_voice(
    &voice.link_state,
    voice.media,
    voice.input_ok,
    voice.output_ok,
    voice.mic_receiving,
  );

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

  // mute/deafen now live in the user controller; the call panel keeps the
  // status readout and the leave button.
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

  // device warnings get their own full-width line below the status/leave row so
  // they have room to wrap instead of being crammed into the single-line meta
  // row next to the channel name, latency, and leave button.
  let mut body =
    column![row![status, leave_button].align_y(Center)].spacing(u32::from(SPACE_GRID) / 2);
  if let Some(hint) = media_hint {
    body = body.push(
      container(
        row![
          icon(GoogleMaterialSymbols::Warning).size(14),
          text(hint)
            .size(12)
            .wrapping(text::Wrapping::Word)
            .width(Length::Fill),
        ]
        .spacing(6)
        .align_y(Center),
      )
      .width(Length::Fill)
      .padding([SPACE_GRID / 2, SPACE_GRID])
      .style(|theme: &Theme| {
        let warning = theme.extended_palette().warning;
        container::Style {
          background: Some(warning.weak.color.into()),
          text_color: Some(warning.weak.text),
          border: border::rounded(2),
          ..container::Style::default()
        }
      }),
    );
  }

  let panel = container(body)
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

/// The right-click mixer popup for one remote participant: a name header, a
/// 0–200% volume slider, and a mute toggle. The displayed values come from the
/// caller's persisted pref; each interaction emits a message the top-level
/// update persists and forwards to the voice actor.
fn user_mixer_overlay<'a>(
  user_id: Uuid,
  name: String,
  pref: chat_core::voice_settings::UserAudioPref,
) -> Element<'a, Message> {
  let percent = (pref.volume * 100.0).round() as u32;

  let volume_slider = slider(0.0..=2.0, pref.volume, move |volume| {
    Message::SetUserVolume { user_id, volume }
  })
  .step(0.01)
  .on_release(Message::UserVolumeReleased { user_id });

  let mute_label = if pref.muted { "Unmute" } else { "Mute" };
  let mute_button = button(text(mute_label).size(14))
    .on_press(Message::ToggleUserMute { user_id })
    .width(Length::Fill)
    .style(move |theme: &Theme, status| {
      let palette = theme.extended_palette();
      let background = match status {
        button::Status::Hovered | button::Status::Pressed => palette.background.strong.color,
        _ => palette.background.weakest.color,
      };
      button::Style {
        background: Some(background.into()),
        text_color: if pref.muted {
          palette.warning.base.color
        } else {
          palette.background.base.text
        },
        border: Border {
          radius: (SPACE_GRID as u32 / 2).into(),
          ..Default::default()
        },
        ..button::Style::default()
      }
    });

  container(
    column![
      text(name)
        .font(Font {
          weight: Weight::Semibold,
          ..SOURCE_SANS_REGULAR
        })
        .size(14),
      row![
        text("Volume").size(12),
        iced::widget::space::horizontal(),
        text(format!("{percent}%")).size(12),
      ]
      .align_y(Center),
      volume_slider,
      mute_button,
    ]
    .spacing(SPACE_GRID as u32)
    .width(180),
  )
  .padding(SPACE_GRID * 2)
  .style(|theme: &Theme| {
    let palette = theme.extended_palette();
    container::Style {
      background: Some(palette.background.weak.color.into()),
      border: Border {
        color: palette.background.strong.color,
        width: 1.0,
        radius: border::radius(SPACE_GRID as u32),
      },
      ..container::Style::default()
    }
  })
  .into()
}
