use std::str::FromStr;

use crate::screens::chat::View::NoneSelected;
use crate::{Element, chat_stream, client};
use crate::{SPACE_GRID, model::Stream};

use crate::types::async_data::AsyncData;
use chat_shared::convert::{IntoProto, TryIntoDomain};
use chat_shared::domain::post::{GetPostsRequest, GetPostsResponse, Post};
use chat_shared::domain::server::{ChannelType, Server, ServersResponse};
use chat_shared::domain::stream::{ClientText, ServerText, User};
use chrono::{Local, Utc};
use iced::widget::keyed::column;
use iced::widget::{column, operation, scrollable, text, text_input};
use iced::widget::{container, row};
use iced::{Border, Length, Pixels, Task};
use indexmap::IndexMap;
use uuid::Uuid;

// --------------------------------- MODEL ---------------------------------

#[derive(Default)]
pub struct Model {
  servers: AsyncData<Vec<Server>, tonic::Status>,
  view: View,
  input: String,
}

// todo: view should model which server we're in, then which text channel we're in.
#[derive(Default)]
enum View {
  #[default]
  NoneSelected,
  TextChannel(TextChannel),
}

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

        let text_channel = global_server
          .channels
          .into_iter()
          .find(|channel| channel.r#type == ChannelType::Text);

        if let Some(text_channel) = text_channel {
          Task::done(Message::UserSelectedTextChannel {
            text_channel_id: text_channel.id,
            name: text_channel.name,
          })
        } else {
          Task::none()
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
  }
}

fn make_text_input_id(text_channel_id: &Uuid) -> String {
  format!("text_input_{}", { text_channel_id.to_string() })
}

// --------------------------------- VIEW ---------------------------------

pub fn view(model: &Model) -> Element<'_, Message> {
  let servers_loaded = match &model.servers {
    AsyncData::NotAsked | AsyncData::Loading => Some("Loading servers...".to_string()),
    AsyncData::Done(Ok(_)) => None,
    AsyncData::Done(Err(status)) => Some(format!(
      "An error occurred while loading servers: {}",
      status.code()
    )),
  };

  if let Some(loading_msg) = servers_loaded {
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

  column![text_chat].spacing({ SPACE_GRID / 2 } as u32).into()
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
    row![view_posts(posts, loading_more)].padding([SPACE_GRID, 0]),
    // row: text posts - user list
    text_input(&format!("Message #{name}"), text_input_string)
      .id(make_text_input_id(text_channel_id))
      .on_input(Message::UserChangedChatInput)
      .on_submit(Message::UserSubmittedChatInput)
      .style(|theme, status| {
        let default_style = text_input::default(theme, status);
        text_input::Style {
          border: Border {
            radius: ((SPACE_GRID / 2) as u32).into(),
            ..default_style.border
          },
          ..default_style
        }
      })
      .padding(SPACE_GRID),
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
  container(text(format!("#{name}")).size(16))
    .width(Length::Fill)
    .style(container::secondary)
    .padding([SPACE_GRID / 2, SPACE_GRID])
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
          text(content).style(text_color)
        ]
        .spacing(Pixels(SPACE_GRID.into()))
        .into(),
      )
    })
    .collect::<Vec<(Uuid, Element<'a, Message>)>>();

  children.extend(posts);

  scrollable(column::Column::with_children(children))
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
