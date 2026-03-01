pub enum AsyncData<T, E> {
  NotAsked,
  Loading,
  Done(Result<T, E>),
}

impl<A, E> AsyncData<A, E> {
  pub fn map<Fun, B>(self, handler: Fun) -> AsyncData<B, E>
  where
    Fun: FnOnce(A) -> B,
  {
    match self {
      AsyncData::NotAsked => AsyncData::NotAsked,
      AsyncData::Loading => AsyncData::Loading,
      AsyncData::Done(Ok(value)) => AsyncData::Done(Ok(handler(value))),
      AsyncData::Done(Err(err)) => AsyncData::Done(Err(err)),
    }
  }

  pub fn as_mut(&mut self) -> AsyncData<&mut A, &mut E> {
    match self {
      AsyncData::NotAsked => AsyncData::NotAsked,
      AsyncData::Loading => AsyncData::Loading,
      AsyncData::Done(Ok(value)) => AsyncData::Done(Ok(value)),
      AsyncData::Done(Err(err)) => AsyncData::Done(Err(err)),
    }
  }

  pub fn as_ref(&self) -> AsyncData<&A, &E> {
    match self {
      AsyncData::NotAsked => AsyncData::NotAsked,
      AsyncData::Loading => AsyncData::Loading,
      AsyncData::Done(Ok(value)) => AsyncData::Done(Ok(value)),
      AsyncData::Done(Err(err)) => AsyncData::Done(Err(err)),
    }
  }

  pub fn and_then<Fun, B>(self, handler: Fun) -> AsyncData<B, E>
  where
    Fun: Fn(A) -> AsyncData<B, E>,
  {
    match self {
      AsyncData::NotAsked => AsyncData::NotAsked,
      AsyncData::Loading => AsyncData::Loading,
      AsyncData::Done(Ok(value)) => handler(value),
      AsyncData::Done(Err(err)) => AsyncData::Done(Err(err)),
    }
  }

  pub fn get_or(self, or: A) -> A {
    match self {
      AsyncData::Done(Ok(value)) => value,
      _ => or,
    }
  }

  pub fn is_not_asked(&self) -> bool {
    match self {
      AsyncData::NotAsked => true,
      _ => false,
    }
  }

  pub fn is_loading(&self) -> bool {
    match self {
      AsyncData::Loading => true,
      _ => false,
    }
  }

  pub fn is_done(&self) -> bool {
    match self {
      AsyncData::Done(_) => true,
      _ => false,
    }
  }
}
