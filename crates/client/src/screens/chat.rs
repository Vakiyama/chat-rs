use crate::audio_processing::cues::Cue;
use crate::colors::{AccentsExt, NeutralsExt};
use crate::model::{self, LinkState::*, MediaHealth};
use crate::screens::chat::View::NoneSelected;
use crate::types::async_data::AsyncData;
use crate::widgets::context_menu::ContextMenu;
use crate::{Element, SOURCE_SANS_REGULAR, chat_stream, client, icon};
use crate::{SPACE_GRID, model::Stream};
use chat_shared::convert::{IntoProto, TryIntoDomain};
use chat_shared::domain::post::{GetPostsRequest, GetPostsResponse, Post};
use chat_shared::domain::server::{
  Channel, ChannelType, Server, ServersResponse, SetChannelMuteRequest,
};
use chat_shared::domain::stream::{ClientText, ServerText, User};
use chrono::{Local, Utc};
use google_material_symbols::GoogleMaterialSymbols;
use iced::Alignment::Center;
use iced::font::Weight;
use iced::widget::scrollable::Scrollbar;
use iced::widget::{
  button, column, mouse_area, operation, rule, scrollable, slider, space, stack, text, text_editor,
};
use iced::widget::{container, row};
use iced::{Border, Font, Length, Padding, Pixels, Task, Theme, border, padding};
use indexmap::IndexMap;
use notify_rust::Notification;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const TYPING_SEND_THROTTLE: Duration = Duration::from_millis(2500);
const TYPING_TIMEOUT: Duration = Duration::from_secs(5);
use uuid::Uuid;

type TextChannelId = Uuid;
// --------------------------------- MODEL ---------------------------------

#[derive(Default)]
pub struct Model {
  servers: AsyncData<Vec<Server>, tonic::Status>,
  view: View,
  posts: HashMap<TextChannelId, Posts>,
  input: text_editor::Content,
  typing: HashMap<Uuid, IndexMap<Uuid, TypingPeer>>,
  typing_seq: u64,
  last_typing_sent: Option<Instant>,
  editing: Option<Editing>,
  confirming_delete: Option<DeleteTarget>,
  shift_held: bool,
  hovered_post: Option<Uuid>,
}

struct Editing {
  post_id: Uuid,
  text_channel_id: Uuid,
  content: text_editor::Content,
}

#[derive(Clone, Copy)]
struct DeleteTarget {
  post_id: Uuid,
  text_channel_id: Uuid,
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
  name: String,
}

struct Posts {
  posts: AsyncData<IndexMap<Uuid, RenderedPost>, tonic::Status>,
  next_timestamp: Option<chrono::DateTime<Utc>>,
  loading_more: bool,
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
  EditorAction(text_editor::Action),
  UserSubmittedChatInput,
  StartEditPost {
    post_id: Uuid,
    text_channel_id: Uuid,
  },
  EditAction(text_editor::Action),
  SubmitEdit,
  CancelEdit,
  EditLastOwnPost,
  ArrowUpInInput,
  RequestDeletePost {
    post_id: Uuid,
    text_channel_id: Uuid,
  },
  ConfirmDeletePost,
  CancelDeletePost,
  ShiftHeld(bool),
  PostHovered(Uuid),
  PostUnhovered(Uuid),
  EscapePressed,
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
  ToggleChannelMute {
    text_channel_id: Uuid,
  },
  ApiReturnedServers(Result<ServersResponse, tonic::Status>),
  ApiReturnedInitialPosts(Result<GetPostsResponse, tonic::Status>),
  ApiReturnedMorePosts(Result<GetPostsResponse, tonic::Status>),
  UserScrolledToTop,
  TypeAhead(String),
  TypeAheadBackspace,
  Init,
  JoinVoice {
    voice_channel_id: Uuid,
  },
  LeaveVoice,
  ActiveServerChanged {
    server_id: Uuid,
  },
  ToggleMute,
  ToggleDeafen,
  SetUserVolume {
    user_id: Uuid,
    volume: f32,
  },
  UserVolumeReleased {
    user_id: Uuid,
  },
  ToggleUserMute {
    user_id: Uuid,
  },
  GoToSettings,
  PlayCue(Cue),
  None,
}

// --------------------------------- UPDATE ---------------------------------
pub fn update(
  model: &mut Model,
  message: Message,
  user: &User,
  stream: Stream<chat_stream::ChatConnection>,
  window_state: &model::WindowState,
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

      let Some(Posts {
        posts: AsyncData::Done(Ok(posts)),
        ..
      }) = model.posts.get_mut(&text_channel.id)
      else {
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

        let text_channel_id = post.text_channel_id;
        // a delivered post means its author stopped typing. Post carries no user
        // id, so clear by author name
        let author_name = post.author_name.clone();

        let Some(Posts {
          posts: AsyncData::Done(Ok(posts)),
          ..
        }) = model.posts.get_mut(&text_channel_id)
        else {
          return show_text_notif(window_state, model, &text_channel_id, &post);
        };

        // assumption: reconciled posts will be near end of indexmap (recently sent), usually causing a small shift
        // on remove/reinsert
        let (_, removed) =
          posts.insert_sorted_by(post.id, RenderedPost::Sent(post.clone()), |_, v1, _, v2| {
            if v1.created_at() > v2.created_at() {
              std::cmp::Ordering::Greater
            } else {
              std::cmp::Ordering::Less
            }
          });

        if let Some(channel_typing) = model.typing.get_mut(&text_channel_id) {
          channel_typing.retain(|_, peer| peer.name != author_name);
        }
        if model
          .typing
          .get(&text_channel_id)
          .is_some_and(|c| c.is_empty())
        {
          model.typing.remove(&text_channel_id);
        }

        if removed.is_none() {
          show_text_notif(window_state, model, &text_channel_id, &post)
        } else {
          Task::none()
        }
      }
      ServerText::PostEdited {
        id,
        content,
        text_channel_id,
      } => {
        if let Some(Posts {
          posts: AsyncData::Done(Ok(posts)),
          ..
        }) = model.posts.get_mut(&text_channel_id)
          && let Some(RenderedPost::Sent(post)) = posts.get_mut(&id)
        {
          post.content = content;
          post.edited = true;
        }
        Task::none()
      }
      ServerText::PostDeleted {
        id,
        text_channel_id,
      } => {
        if let Some(Posts {
          posts: AsyncData::Done(Ok(posts)),
          ..
        }) = model.posts.get_mut(&text_channel_id)
        {
          posts.shift_remove(&id);
        }
        if model
          .editing
          .as_ref()
          .is_some_and(|editing| editing.post_id == id)
        {
          model.editing = None;
        }
        if model
          .confirming_delete
          .is_some_and(|target| target.post_id == id)
        {
          model.confirming_delete = None;
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
        name,
      });

      if model.posts.contains_key(&text_channel_id) {
        Task::none() // messages were loaded before
      } else {
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
    }
    Message::ToggleChannelMute { text_channel_id } => {
      let AsyncData::Done(Ok(servers)) = &mut model.servers else {
        return Task::none();
      };

      let Some(channel) = servers
        .iter_mut()
        .flat_map(|server| server.channels.iter_mut())
        .find(|channel| channel.id == text_channel_id)
      else {
        return Task::none();
      };

      channel.muted = !channel.muted;
      let muted = channel.muted;

      Task::future(async move {
        let mut client = client::get().await;
        if let Err(status) = client
          .server
          .set_channel_mute(
            SetChannelMuteRequest {
              text_channel_id,
              muted,
            }
            .into_proto(),
          )
          .await
        {
          eprintln!("failed to persist channel mute: {status}");
        }
        Message::None
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
      if let Ok(ref res) = res {
        model.posts.insert(
          res.text_channel_id,
          Posts {
            posts: AsyncData::Done(Ok(IndexMap::from_iter(
              res
                .posts
                .clone()
                .into_iter()
                .map(|post| (post.id, RenderedPost::Sent(post))),
            ))),
            next_timestamp: res.next_timestamp,
            loading_more: false,
          },
        );
      };

      Task::none()
    }
    Message::None => Task::none(),
    Message::ApiReturnedMorePosts(res) => {
      let Ok(res) = res else {
        return Task::none(); // (or stash the error somewhere)
      };

      if let Some(Posts {
        posts: AsyncData::Done(Ok(posts)),
        next_timestamp,
        loading_more,
        ..
      }) = model.posts.get_mut(&res.text_channel_id)
      {
        let existing = std::mem::take(posts);

        *loading_more = false; // release the prefetch guard, success or not
        *next_timestamp = res.next_timestamp;

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

      let Some(posts) = model.posts.get(&text_channel.id) else {
        return Task::none();
      };

      if posts.loading_more || posts.next_timestamp.is_none() {
        return Task::none();
      };

      let text_channel_id = text_channel.id;
      let starting_before_timestamp = posts.next_timestamp;

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
    Message::TypeAheadBackspace => {
      let Some(current_text_channel_id) = (match model.view {
        View::TextChannel(TextChannel { id, .. }) => Some(id),
        _ => None,
      }) else {
        return Task::none();
      };

      let text_input_id = make_text_input_id(&current_text_channel_id);
      model
        .input
        .perform(text_editor::Action::Edit(text_editor::Edit::Backspace));
      operation::focus(text_input_id)
    }
    Message::StartEditPost {
      post_id,
      text_channel_id,
    } => start_editing(model, post_id, text_channel_id),
    Message::EditAction(action) => {
      if let Some(editing) = model.editing.as_mut() {
        editing.content.perform(action);
      }
      Task::none()
    }
    Message::CancelEdit => {
      model.editing = None;
      Task::none()
    }
    Message::SubmitEdit => {
      let Some(editing) = model.editing.take() else {
        return Task::none();
      };
      let content = editing.content.text();
      if content.trim().is_empty() {
        return Task::none();
      }

      if let Some(Posts {
        posts: AsyncData::Done(Ok(posts)),
        ..
      }) = model.posts.get_mut(&editing.text_channel_id)
        && let Some(RenderedPost::Sent(post)) = posts.get_mut(&editing.post_id)
      {
        post.content = content.clone();
        post.edited = true;
      }

      if let Stream::Connected(mut stream) = stream {
        let _ = stream.try_send(ClientText::EditPostRequest {
          id: editing.post_id,
          content,
          text_channel_id: editing.text_channel_id,
        });
      }
      Task::none()
    }
    Message::EditLastOwnPost => {
      if !model.input.text().trim().is_empty() {
        return Task::none();
      }
      match last_own_post(model, user.id) {
        Some((post_id, text_channel_id)) => start_editing(model, post_id, text_channel_id),
        None => Task::none(),
      }
    }
    Message::ArrowUpInInput => {
      if model.input.text().trim().is_empty()
        && let Some((post_id, text_channel_id)) = last_own_post(model, user.id)
      {
        start_editing(model, post_id, text_channel_id)
      } else {
        model
          .input
          .perform(text_editor::Action::Move(text_editor::Motion::Up));
        Task::none()
      }
    }
    Message::RequestDeletePost {
      post_id,
      text_channel_id,
    } => {
      let target = DeleteTarget {
        post_id,
        text_channel_id,
      };
      if model.shift_held {
        delete_post(model, target, stream)
      } else {
        model.confirming_delete = Some(target);
        Task::none()
      }
    }
    Message::ConfirmDeletePost => {
      let Some(target) = model.confirming_delete.take() else {
        return Task::none();
      };
      delete_post(model, target, stream)
    }
    Message::CancelDeletePost => {
      model.confirming_delete = None;
      Task::none()
    }
    Message::ShiftHeld(held) => {
      model.shift_held = held;
      Task::none()
    }
    Message::PostHovered(post_id) => {
      model.hovered_post = Some(post_id);
      Task::none()
    }
    Message::PostUnhovered(post_id) => {
      if model.hovered_post == Some(post_id) {
        model.hovered_post = None;
      }
      Task::none()
    }
    Message::EscapePressed => {
      if model.confirming_delete.take().is_none() {
        model.editing = None;
      }
      Task::none()
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
    | Message::GoToSettings
    | Message::PlayCue(_) => Task::none(),
  }
}

fn make_text_input_id(text_channel_id: &Uuid) -> String {
  format!("text_input_{}", { text_channel_id.to_string() })
}

fn make_edit_input_id(post_id: &Uuid) -> String {
  format!("edit_input_{}", { post_id.to_string() })
}

/// Open the inline editor for a sent post, pre-filling it with the current
/// content and focusing it. No-op if the post isn't a delivered (`Sent`) post.
fn start_editing(model: &mut Model, post_id: Uuid, text_channel_id: Uuid) -> Task<Message> {
  let Some(Posts {
    posts: AsyncData::Done(Ok(posts)),
    ..
  }) = model.posts.get(&text_channel_id)
  else {
    return Task::none();
  };
  let Some(RenderedPost::Sent(post)) = posts.get(&post_id) else {
    return Task::none();
  };

  model.editing = Some(Editing {
    post_id,
    text_channel_id,
    content: text_editor::Content::with_text(&post.content),
  });
  operation::focus(make_edit_input_id(&post_id))
}

/// The most recent own, delivered post in the currently viewed channel.
fn last_own_post(model: &Model, user_id: Uuid) -> Option<(Uuid, Uuid)> {
  let text_channel_id = match model.view {
    View::TextChannel(TextChannel { id, .. }) => id,
    _ => return None,
  };
  let Some(Posts {
    posts: AsyncData::Done(Ok(posts)),
    ..
  }) = model.posts.get(&text_channel_id)
  else {
    return None;
  };
  posts.values().rev().find_map(|post| match post {
    RenderedPost::Sent(post) if post.author_id == user_id => Some((post.id, text_channel_id)),
    _ => None,
  })
}

/// Optimistically remove a post locally and ask the server to delete it.
fn delete_post(
  model: &mut Model,
  target: DeleteTarget,
  stream: Stream<chat_stream::ChatConnection>,
) -> Task<Message> {
  if let Some(Posts {
    posts: AsyncData::Done(Ok(posts)),
    ..
  }) = model.posts.get_mut(&target.text_channel_id)
  {
    posts.shift_remove(&target.post_id);
  }
  if model
    .editing
    .as_ref()
    .is_some_and(|e| e.post_id == target.post_id)
  {
    model.editing = None;
  }

  if let Stream::Connected(mut stream) = stream {
    let _ = stream.try_send(ClientText::DeletePostRequest {
      id: target.post_id,
      text_channel_id: target.text_channel_id,
    });
  }
  Task::none()
}

fn show_text_notif(
  window_state: &model::WindowState,
  model: &Model,
  text_channel_id: &Uuid,
  post: &Post,
) -> Task<Message> {
  if ((matches!(window_state, model::WindowState::NotFocused))
    || !matches!(
      &model.view,
      View::TextChannel(TextChannel {
        id,
        name: _
      }) if id == text_channel_id
    ))
    && let AsyncData::Done(Ok(servers)) = &model.servers
    && let Some(text_channel) = servers.first().and_then(|server| {
      server.channels.iter().find(|channel| {
        matches!(channel.r#type, ChannelType::Text) && channel.id == *text_channel_id
      })
    })
    && !text_channel.muted
  {
    // todo: assumes 1 server
    let _ = Notification::new()
      .summary(&format!(
        "{}\n({}, #{})",
        post.author_name,
        &servers.first().unwrap().name,
        &text_channel.name
      ))
      .body(&post.content)
      .show()
      .map_err(|err| eprintln!("{err}"));

    Task::done(Message::PlayCue(Cue::Message))
  } else {
    Task::none()
  }
}

// --------------------------------- VIEW ---------------------------------

pub fn view<'a>(
  model: &'a Model,
  current_call_id: Option<&'a Uuid>,
  main_model: &'a crate::model::Model,
) -> Element<'a, Message> {
  let current_user_id = match &main_model.user {
    model::Auth::LoggedIn(user) => Some(user.id),
    model::Auth::NotLoggedIn => None,
  };
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
    View::TextChannel(text_channel) => match &model.posts.get(&text_channel.id) {
      Some(Posts {
        posts: AsyncData::NotAsked,
        ..
      })
      | Some(Posts {
        posts: AsyncData::Loading,
        ..
      })
      | None => {
        // todo: Spinner here
        container(
          text("Loading posts...")
            .center()
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .into()
      }
      Some(Posts {
        posts: AsyncData::Done(Err(status)),
        ..
      }) => container(
        text(format!(
          "An error occurred while loading posts: {}",
          status.code()
        ))
        .center()
        .width(Length::Fill),
      )
      .into(),
      Some(Posts {
        posts: AsyncData::Done(Ok(posts)),
        loading_more,
        ..
      }) => {
        let typing_names: Vec<&str> = model
          .typing
          .get(&text_channel.id)
          .map(|peers| peers.values().map(|p| p.name.as_str()).collect())
          .unwrap_or_default();
        view_text_chat_window(
          model,
          &text_channel.name,
          posts,
          *loading_more,
          &text_channel.id,
          typing_names,
          current_user_id,
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
  let base: Element<'_, Message> = row![
    view_channels(
      model,
      "The Intergalactic Federation",
      channels,
      current_call_id,
      main_model
    ),
    text_chat
  ]
  .into();

  match model.confirming_delete {
    Some(_) => stack![base, delete_confirm_dialog()].into(),
    None => base,
  }
}

fn view_channels<'a>(
  model: &'a Model,
  server_name: &'a str,
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
      let is_selected = match &model.view {
        NoneSelected => false,
        View::TextChannel(text_channel_view) => text_channel.id == text_channel_view.id,
      };
      let is_muted = text_channel.muted;
      let text_channel_id = text_channel.id;

      let channel_button = button(
        row![
          icon(GoogleMaterialSymbols::Tag)
            .size(20)
            .style(move |theme| {
              let color = theme.extended_palette().background.weakest.text;
              text::Style {
                color: Some(if is_muted {
                  color.scale_alpha(0.45)
                } else {
                  color
                }),
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

        let text_color = if is_selected || matches!(status, button::Status::Hovered) {
          palette.background.neutral.text
        } else {
          palette.background.weakest.text
        };

        button::Style {
          background,
          text_color: if is_muted {
            text_color.scale_alpha(0.45)
          } else {
            text_color
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
      });

      // Right-click a text channel to mute/unmute it, mirroring the in-call
      // right-click mixer on voice participants.
      ContextMenu::new(channel_button, move || {
        channel_mute_overlay(text_channel_id, is_muted)
      })
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
        .padding(SPACE_GRID / 2)
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
      .spacing((SPACE_GRID as u32) / 2);
      //      .padding([
      //        if main_model
      //          .room_presence
      //          .get(&voice_channel.id)
      //          .is_some_and(|presence| !presence.is_empty())
      //        {
      //          SPACE_GRID / 2
      //        } else {
      //          0
      //        },
      //        0,
      //      ]);

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
  // controller rather than only inside the active-call panel.
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
            weight: Weight::Bold,
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
  model: &'a Model,
  name: &'a str,
  posts: &'a IndexMap<Uuid, RenderedPost>,
  loading_more: bool,
  text_channel_id: &'a Uuid,
  typing_names: Vec<&'a str>,
  current_user_id: Option<Uuid>,
) -> Element<'a, Message> {
  let editor = text_editor(&model.input)
    .placeholder(format!("Message #{name}"))
    .id(make_text_input_id(text_channel_id))
    .on_action(Message::EditorAction)
    .key_binding(|key_press| {
      use iced::keyboard::Key;
      use iced::keyboard::key::Named;
      use text_editor::{Binding, Motion};

      let modifier = key_press.modifiers.command() || key_press.modifiers.control();
      let is_char = |c| matches!(key_press.key.as_ref(), Key::Character(s) if s == c);
      let is_backspace = matches!(&key_press.key, Key::Named(Named::Backspace));

      let plain = !modifier && !key_press.modifiers.shift() && !key_press.modifiers.alt();

      if matches!(&key_press.key, Key::Named(Named::Enter)) && !key_press.modifiers.shift() {
        Some(Binding::Custom(Message::UserSubmittedChatInput))
      } else if plain && matches!(&key_press.key, Key::Named(Named::ArrowUp)) {
        // empty composer + up = edit last own message; otherwise cursor up.
        // The handler decides based on the buffer contents.
        Some(Binding::Custom(Message::ArrowUpInInput))
      } else if modifier && is_char("u") {
        Some(Binding::Sequence(vec![
          Binding::SelectLine,
          Binding::Backspace,
        ]))
      } else if modifier && (is_char("w") || is_backspace) {
        Some(Binding::Sequence(vec![
          Binding::Select(Motion::WordLeft),
          Binding::Backspace,
        ]))
      } else {
        Binding::from_key_press(key_press)
      }
    })
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
    stack![
      row![view_posts(
        model,
        posts,
        loading_more,
        text_channel_id,
        current_user_id,
      )]
      .padding([0, SPACE_GRID * 2]),
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

fn view_typing_indicator<'a>(names: &[&str]) -> Element<'a, Message> {
  let line = match names {
    [] => None,
    [a] => Some(format!("{a} is typing…")),
    [a, b] => Some(format!("{a} and {b} are typing…")),
    [a, b, c] => Some(format!("{a}, {b}, and {c} are typing…")),
    _ => Some("Several people are typing…".to_string()),
  };

  match line {
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

fn view_day_divider<'a>(date: chrono::NaiveDate) -> Element<'a, Message> {
  let today = Local::now().date_naive();
  let label = match (today - date).num_days() {
    0 => "Today".to_string(),
    1 => "Yesterday".to_string(),
    _ => date.format("%A, %B %-d, %Y").to_string(),
  };

  let line = || {
    rule::horizontal(1).style(|theme: &Theme| rule::Style {
      color: theme.extended_palette().secondary.weak.color,
      ..rule::default(theme)
    })
  };

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
  .into()
}

fn view_posts<'a>(
  model: &'a Model,
  posts: &'a IndexMap<Uuid, RenderedPost>,
  loading_more: bool,
  text_channel_id: &'a Uuid,
  current_user_id: Option<Uuid>,
) -> Element<'a, Message> {
  // NOTE: this is a regular (non-keyed) `column`. `keyed::column` must NOT be
  // used here: its diff reuses child state trees by key *without* a tag check
  // (`child.diff(tree)` directly), so a key whose element changes type — or a
  // mid-list insert/remove that shifts a stateful child (`ContextMenu`, the
  // inline editor) onto a stale tree — hands `State::None` to a stateful widget
  // and panics with "Downcast on stateless state". A regular column diffs
  // positionally through `Tree::diff`, which rebuilds on tag mismatch.
  let mut children: Vec<Element<'a, Message>> = vec![];
  if loading_more {
    children.push(text("Loading more posts...").width(Length::Fill).into());
  };

  const MESSAGE_GROUP_GAP: chrono::TimeDelta = chrono::TimeDelta::minutes(5);
  const MESSAGE_GROUP_MAX: usize = 10;

  let mut previous_date: Option<chrono::NaiveDate> = None;
  let mut previous_author: Option<&str> = None;
  let mut previous_created: Option<chrono::DateTime<Utc>> = None;
  let mut run_length: usize = 0;
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

    let gap_exceeded = previous_created
      .map(|prev| *created_at - prev >= MESSAGE_GROUP_GAP)
      .unwrap_or(true);
    let show_name = is_new_day
      || previous_author != Some(name.as_str())
      || gap_exceeded
      || run_length >= MESSAGE_GROUP_MAX;

    run_length = if show_name { 1 } else { run_length + 1 };
    let display_name = if show_name {
      name.clone()
    } else {
      String::new()
    };
    previous_author = Some(name);
    previous_created = Some(*created_at);

    let display_time = local.format("%I:%M %p").to_string();
    let text_color = match post.1 {
      RenderedPost::Sending { .. } => text::secondary,
      RenderedPost::Errored { .. } => text::danger,
      RenderedPost::Sent(_) => text::default,
    };

    let content_text = iced_selection::text(content)
      .style(move |theme| iced_selection::text::Style {
        color: text_color(theme).color,
        selection: theme.extended_palette().secondary.strong.text,
      })
      .wrapping(text::Wrapping::WordOrGlyph);

    // an edited post gets a small muted "(edited)" marker trailing its content.
    let is_edited = matches!(post.1, RenderedPost::Sent(p) if p.edited);
    let content_block: Element<'a, Message> = if is_edited {
      row![
        content_text,
        text("(edited)")
          .size(11)
          .style(|theme: &Theme| text::Style {
            color: Some(theme.extended_palette().background.weak.text),
          })
      ]
      .spacing(SPACE_GRID as u32 / 2)
      .align_y(iced::Alignment::End)
      .into()
    } else {
      content_text.into()
    };

    let body: Element<'a, Message> = if display_name.is_empty() {
      content_block
    } else {
      row![
        iced_selection::text(display_name)
          .style(|theme| iced_selection::text::Style {
            color: text::base(theme).color,
            selection: theme.extended_palette().secondary.strong.text
          })
          .font(Font {
            weight: Weight::Semibold,
            ..SOURCE_SANS_REGULAR
          }),
        content_block
      ]
      .spacing(Pixels(SPACE_GRID.into()))
      .into()
    };

    let top_pad = if show_name && !is_new_day {
      SPACE_GRID as f32
    } else {
      0.0
    };

    // only the author's own delivered posts get the edit/delete affordances.
    // (Sending/Errored posts have no server-side id to act on yet.)
    let is_own = matches!(post.1, RenderedPost::Sent(p) if Some(p.author_id) == current_user_id);
    let is_editing = model
      .editing
      .as_ref()
      .is_some_and(|e| &e.post_id == id && &e.text_channel_id == text_channel_id);
    let is_hovered = model.hovered_post == Some(*id);

    // The first message in a group always shows its timestamp; grouped messages
    // hide it (rendered transparent so the gutter width is reserved and content
    // never shifts) and reveal it on hover.
    let timestamp_shown = show_name || is_hovered;
    let timestamp =
      iced_selection::text(display_time).style(move |theme| iced_selection::text::Style {
        color: if timestamp_shown {
          text::secondary(theme).color
        } else {
          Some(iced::Color::TRANSPARENT)
        },
        selection: theme.extended_palette().secondary.strong.text,
      });

    let line: Element<'a, Message> = if is_editing {
      // SAFETY: is_editing implies editing is Some.
      let content = &model.editing.as_ref().unwrap().content;
      row![timestamp, view_inline_editor(content, *id)]
        .spacing(Pixels(SPACE_GRID.into()))
        .into()
    } else {
      row![timestamp, body]
        .spacing(Pixels(SPACE_GRID.into()))
        .into()
    };

    // Full-width hover highlight signalling the row is interactive (context menu
    // / replies). Driven by model state via the mouse_area below. Sized tight to
    // the content (no extra padding, to avoid shifting layout vs. the day
    // dividers); the inter-group gap (`top_pad`) is applied *outside* it so every
    // row's highlight is the same height.
    let highlighted = container(line)
      .width(Length::Fill)
      .style(move |theme: &Theme| container::Style {
        background: is_hovered.then(|| theme.neutrals().surface0.into()),
        border: Border {
          radius: (SPACE_GRID as u32 / 2).into(),
          ..Default::default()
        },
        ..container::Style::default()
      });

    // Wrap the *full-width* highlight (not just the content row) so right-click
    // works anywhere on the hovered line, not only over the text. No menu while
    // editing.
    let with_menu: Element<'a, Message> = if is_own && !is_editing {
      let post_id = *id;
      let channel_id = *text_channel_id;
      ContextMenu::new(highlighted, move || {
        message_context_menu(post_id, channel_id)
      })
      .close_on_release(true)
      .into()
    } else {
      highlighted.into()
    };

    let post_id = *id;
    let hoverable = mouse_area(with_menu)
      .on_enter(Message::PostHovered(post_id))
      .on_exit(Message::PostUnhovered(post_id));

    children.push(
      container(hoverable)
        .width(Length::Fill)
        .padding(padding::top(top_pad))
        .into(),
    );
  }

  let scrollbar = Scrollbar::new().width(4).scroller_width(4);

  scrollable(
    iced::widget::Column::with_children(children).padding(padding::Padding {
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

fn describe_voice(
  link: &crate::model::LinkState,
  media: crate::model::MediaHealth,
  input_ok: bool,
  output_ok: bool,
  mic_receiving: bool,
) -> (String, Tone, Option<String>) {
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
    Reconnecting { attempt: _ } => ("Reconnecting...".into(), Tone::Warn, None),
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

  let reconnecting = main_model
    .pending_rejoin
    .as_ref()
    .is_some_and(|r| &r.voice_channel_id == voice_channel_id);

  let (status_text, tone, media_hint, connected, latency_ms) = match main_model.voice.as_ref() {
    Some(voice) => {
      let (status_text, tone, media_hint) = describe_voice(
        &voice.link_state,
        voice.media,
        voice.input_ok,
        voice.output_ok,
        voice.mic_receiving,
      );
      let connected = matches!(
        voice.link_state,
        crate::model::LinkState::Live | crate::model::LinkState::Unstable
      );
      (status_text, tone, media_hint, connected, voice.latency_ms)
    }
    None if reconnecting => ("Reconnecting…".to_string(), Tone::Warn, None, false, 0),
    None => return None,
  };

  // secondary line: channel name, then latency + media hint only while connected
  let mut meta = row![
    text(channel_name)
      .size(12)
      .style(text::secondary)
      .wrapping(text::Wrapping::None)
  ]
  .spacing(6)
  .align_y(Center);

  if connected && latency_ms > 0 {
    let lt = latency_tone(latency_ms);
    meta = meta.push(text("·").size(12).style(text::secondary));
    meta = meta.push(
      text(format!("{latency_ms} ms"))
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

fn channel_mute_overlay<'a>(text_channel_id: Uuid, muted: bool) -> Element<'a, Message> {
  let (label, symbol) = if muted {
    ("Unmute", GoogleMaterialSymbols::Notifications)
  } else {
    ("Mute", GoogleMaterialSymbols::NotificationsOff)
  };

  let mute_button = button(
    row![icon(symbol).size(16), text(label).size(14)]
      .spacing(SPACE_GRID as u32)
      .align_y(Center),
  )
  .on_press(Message::ToggleChannelMute { text_channel_id })
  .width(Length::Fill)
  .style(move |theme: &Theme, status| {
    let palette = theme.extended_palette();
    let background = match status {
      button::Status::Hovered | button::Status::Pressed => palette.background.strong.color,
      _ => palette.background.weakest.color,
    };
    button::Style {
      background: Some(background.into()),
      text_color: palette.background.base.text,
      border: Border {
        radius: (SPACE_GRID as u32 / 2).into(),
        ..Default::default()
      },
      ..button::Style::default()
    }
  });

  container(container(mute_button).width(180))
    .padding(SPACE_GRID)
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

fn user_mixer_overlay<'a>(
  user_id: Uuid,
  name: String,
  pref: crate::voice_settings::UserAudioPref,
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

/// The inline editor shown in place of a post while it's being edited. Enter
/// saves, Shift+Enter inserts a newline, Escape cancels.
fn view_inline_editor<'a>(
  content: &'a text_editor::Content,
  post_id: Uuid,
) -> Element<'a, Message> {
  let editor = text_editor(content)
    .id(make_edit_input_id(&post_id))
    .on_action(Message::EditAction)
    .key_binding(|key_press| {
      use iced::keyboard::Key;
      use iced::keyboard::key::Named;
      use text_editor::Binding;

      match &key_press.key {
        Key::Named(Named::Enter) if !key_press.modifiers.shift() => {
          Some(Binding::Custom(Message::SubmitEdit))
        }
        Key::Named(Named::Escape) => Some(Binding::Custom(Message::CancelEdit)),
        _ => Binding::from_key_press(key_press),
      }
    })
    .padding(SPACE_GRID)
    .style(|theme, status| {
      let default_style = text_editor::default(theme, status);
      text_editor::Style {
        border: Border {
          radius: (SPACE_GRID as u32 / 2).into(),
          ..default_style.border
        },
        ..default_style
      }
    });

  // a clickable text link styled to read as a hint: muted by default, the
  // verb brightening on hover. `word` is the clickable action verb.
  let hint_link = |prefix: &'a str, word: &'a str, message: Message| {
    button(
      row![
        text(prefix).size(11).style(|theme: &Theme| text::Style {
          color: Some(theme.extended_palette().background.weak.text),
        }),
        text(word).size(11).style(|theme: &Theme| text::Style {
          color: Some(theme.extended_palette().background.base.text),
        }),
      ]
      .spacing(SPACE_GRID as u32 / 4),
    )
    .on_press(message)
    .padding(0)
    .style(|_theme: &Theme, _status| button::Style {
      background: None,
      ..button::Style::default()
    })
  };

  let hint = row![
    hint_link("escape to ", "cancel", Message::CancelEdit),
    text("•").size(11).style(|theme: &Theme| text::Style {
      color: Some(theme.extended_palette().background.weak.text),
    }),
    hint_link("enter to ", "save", Message::SubmitEdit),
  ]
  .spacing(SPACE_GRID as u32 / 2)
  .align_y(Center);

  column![editor, hint]
    .spacing(SPACE_GRID as u32 / 2)
    .width(Length::Fill)
    .into()
}

/// The right-click menu for an own message: edit and delete actions.
fn message_context_menu<'a>(post_id: Uuid, text_channel_id: Uuid) -> Element<'a, Message> {
  let menu_button = |label: &'a str, symbol, message: Message, danger: bool| {
    button(
      row![icon(symbol).size(16), text(label).size(14)]
        .spacing(SPACE_GRID as u32)
        .align_y(Center),
    )
    .on_press(message)
    .width(Length::Fill)
    .style(move |theme: &Theme, status| {
      let palette = theme.extended_palette();
      let background = match status {
        button::Status::Hovered | button::Status::Pressed => palette.background.strong.color,
        _ => palette.background.weakest.color,
      };
      button::Style {
        background: Some(background.into()),
        text_color: if danger {
          palette.danger.base.color
        } else {
          palette.background.base.text
        },
        border: Border {
          radius: (SPACE_GRID as u32 / 2).into(),
          ..Default::default()
        },
        ..button::Style::default()
      }
    })
  };

  container(
    column![
      menu_button(
        "Edit",
        GoogleMaterialSymbols::Edit,
        Message::StartEditPost {
          post_id,
          text_channel_id,
        },
        false,
      ),
      menu_button(
        "Delete",
        GoogleMaterialSymbols::Delete,
        Message::RequestDeletePost {
          post_id,
          text_channel_id,
        },
        true,
      ),
    ]
    .spacing(SPACE_GRID as u32 / 2)
    .width(160),
  )
  .padding(SPACE_GRID)
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

/// A modal asking the user to confirm deleting a message. A translucent
/// backdrop dismisses on click; the card offers Cancel / Delete.
fn delete_confirm_dialog<'a>() -> Element<'a, Message> {
  let backdrop = mouse_area(
    container(space::horizontal())
      .width(Length::Fill)
      .height(Length::Fill)
      .style(|_theme: &Theme| container::Style {
        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
        ..container::Style::default()
      }),
  )
  .on_press(Message::CancelDeletePost);

  let cancel = button(text("Cancel").size(14))
    .on_press(Message::CancelDeletePost)
    .style(|theme: &Theme, status| {
      let palette = theme.extended_palette();
      let background = match status {
        button::Status::Hovered | button::Status::Pressed => palette.background.strong.color,
        _ => palette.background.weak.color,
      };
      button::Style {
        background: Some(background.into()),
        text_color: palette.background.base.text,
        border: Border {
          radius: (SPACE_GRID as u32 / 2).into(),
          ..Default::default()
        },
        ..button::Style::default()
      }
    });

  let confirm = button(text("Delete").size(14))
    .on_press(Message::ConfirmDeletePost)
    .style(|theme: &Theme, status| {
      let palette = theme.extended_palette();
      let background = match status {
        button::Status::Hovered | button::Status::Pressed => palette.danger.strong.color,
        _ => palette.danger.base.color,
      };
      button::Style {
        background: Some(background.into()),
        text_color: palette.danger.base.text,
        border: Border {
          radius: (SPACE_GRID as u32 / 2).into(),
          ..Default::default()
        },
        ..button::Style::default()
      }
    });

  let card = container(
    column![
      text("Delete Message").size(16).font(Font {
        weight: Weight::Semibold,
        ..SOURCE_SANS_REGULAR
      }),
      text("Are you sure you want to delete this message? This cannot be undone.").size(13),
      text("Tip: hold Shift while deleting to skip this confirmation.")
        .size(11)
        .style(|theme: &Theme| text::Style {
          color: Some(theme.extended_palette().background.weak.text),
        }),
      row![space::horizontal(), cancel, confirm].spacing(SPACE_GRID as u32),
    ]
    .spacing((SPACE_GRID * 2) as u32)
    .width(360),
  )
  .padding(SPACE_GRID * 3)
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
  });

  stack![
    backdrop,
    container(card).center(Length::Fill).padding(SPACE_GRID * 2)
  ]
  .into()
}
