use iced::Element;
use iced::Subscription;
use iced::keyboard;
use iced::keyboard::Event;
use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::widget::{Column, text, text_input};

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

struct Post {
  content: String,
}

impl Post {
  fn new(content: &str) -> Self {
    Self {
      content: content.to_string(),
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
  let mut children = model
    .posts
    .iter()
    .map(|post| {
      let element: Element<Message> = text(post.content.clone()).into();
      element
    })
    .collect::<Vec<_>>();

  children.extend(vec![
    text_input("Send message", &model.input)
      .on_input(Message::ContentChanged)
      .into(),
  ]);

  Column::with_children(children).into()
}
