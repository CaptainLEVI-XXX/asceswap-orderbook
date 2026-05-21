mod error;
mod lifecycle;
mod reservation;

pub use error::StateError;
pub use lifecycle::{is_allowed_transition, OrderLifecycle, OrderState, OrderTransition};
pub use reservation::{
    derive_reservation_id, OrderAvailability, Reservation, ReservationBook, ReservationId,
    ReservationLeg, ReservationLegRole, ReservationStatus,
};

#[cfg(test)]
mod tests;
