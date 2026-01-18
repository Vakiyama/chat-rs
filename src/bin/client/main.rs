use iced::Element;
use iced::Padding;
use iced::Subscription;
use iced::keyboard;
use iced::keyboard::Event;
use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::widget::container;
use iced::widget::{Column, column, text, text_input};

use chat_rs::schema::post::Model as Post;

mod websocket;

const SPACE_GRID: u16 = 8;

pub fn main() -> iced::Result {
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
      posts: vec![Post::new("Post 1"), Post::new("Post 2")],
      input: String::new(),
    }
  }
}

// TEMPORARY
enum PostError {
  Unknown,
}

#[derive(Clone)]
enum Message {
  ContentChanged(String),
  Keyboard(Event),
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
        posts.push(Post::new(input));
        model.input = "".to_string();
      }
    }
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
      let element: Element<Message> = text(post.content.clone()).into();
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
