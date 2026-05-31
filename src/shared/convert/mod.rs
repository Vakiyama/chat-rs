pub mod auth;
pub mod stream;

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

pub trait TryIntoDomain<D>: Sized {
  type Error;
  fn try_into_domain(self) -> Result<D, Self::Error>;
}

impl<P, D: TryFromProto<P>> TryIntoDomain<D> for P {
  type Error = D::Error;
  fn try_into_domain(self) -> Result<D, Self::Error> {
    D::try_from_proto(self)
  }
}
