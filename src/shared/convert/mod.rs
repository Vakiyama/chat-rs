pub mod auth;

// bridge: impl into status for errors, fix rest of compiler errors
pub trait IntoStatus {
  fn into_status(self) -> tonic::Status;
}

pub trait IntoProto<P> {
  fn into_proto(self) -> P;
}

pub trait TryFromProto<P>: Sized {
  type Error;
  fn try_from_proto(proto: P) -> Result<Self, Self::Error>;
}

pub trait TryIntoProto<P>: Sized {
  type Error;
  fn try_into_proto(self) -> Result<P, Self::Error>;
}

impl<P, D: TryFromProto<P>> TryIntoProto<D> for P {
  type Error = D::Error;
  fn try_into_proto(self) -> Result<D, Self::Error> {
    D::try_from_proto(self)
  }
}
