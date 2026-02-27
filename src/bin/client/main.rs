use iced::Element;
use iced::Subscription;
use iced::keyboard;
use iced::keyboard::Event;
use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::widget::container;
use iced::widget::{Column, column, row, text, text_input};

use chat_rs::schema::post::Model as Post;

mod message;
mod websocket;

use message::Message;

const SPACE_GRID: u16 = 8;

#[tokio::main]
pub async fn main() -> iced::Result {
  iced::application(new, update, view)
    .subscription(subscription)
    .run()
}

fn new() -> Model {
  Model::default()
}

fn subscription(_model: &Model) -> Subscription<Message> {
  keyboard::listen().map(Message::Keyboard)
}

struct Model {
  posts: Vec<Post>,
  input: String,
}

impl Default for Model {
  fn default() -> Self {
    Model {
      posts: vec![
        Post::new("Post 1", "RootPoison"),
        Post::new("Post 2", "Cecilian"),
      ],
      input: String::new(),
    }
  }
}

fn update(model: &mut Model, message: Message) {
  match message {
    Message::ContentChanged(new) => {
      model.input = new;
    }
    Message::Keyboard(event) => {
      if let Event::KeyPressed {
        key: Key::Named(Named::Enter),
        ..
      } = event
      {
        let Model { input, posts } = model;
        posts.push(Post::new(input, "RootPoison"));
        model.input = "".to_string();
      }
    }
    Message::Disconnected => todo!(),
    Message::Connected(_connection) => todo!(),
    Message::Websocket(_server_message) => todo!(),
  }
}

fn view(model: &'_ Model) -> Element<'_, Message> {
  container(
    container(view_chat(model, "#general"))
      .padding(SPACE_GRID)
      .style(container::rounded_box),
  )
  .padding(SPACE_GRID)
  .into()
}

fn view_chat<'a>(model: &'_ Model, chat_title: &'a str) -> Element<'a, Message> {
  let posts = model
    .posts
    .iter()
    .map(|post| {
      let element: Element<Message> =
        row![text(post.author_name.clone()), text(post.content.clone())]
          .spacing(SPACE_GRID as u32)
          .into();
      element
    })
    .collect::<Vec<_>>();

  let children: Element<'_, Message> = column![
    container(Column::with_children(posts))
      .padding([SPACE_GRID, 0])
      .height(iced::Fill),
    text_input("Send message", &model.input)
      .on_input(Message::ContentChanged)
      .padding(SPACE_GRID)
  ]
  .into();

  column![container(chat_title), children]
    .spacing({ SPACE_GRID } as u32)
    .into()
}
