mod actor_service;
mod error;
mod event;
mod request;
mod response;
mod service;
mod wire;

pub use actor_service::{
    spawn_actor_orderbook_api_service, spawn_actor_orderbook_api_service_with_capacity,
    ActorOrderbookApiHandle, ActorOrderbookApiService,
};
pub use error::ApiError;
pub use event::{ApiEvent, ApiEventKind};
pub use request::{
    CancelOrderRequest, MarketDepthRequest, OrderStatusRequest, ReservationActionRequest,
    SettlementPayloadRequest, SubmitOrderRequest, ValidationContextRequest,
};
pub use response::{
    CancelOrderResponse, DepthLevelResponse, MarketDepthResponse, OrderStatusResponse,
    ReservationActionResponse, SettlementPayloadResponse, SubmitOrderResponse,
    SubmitOrderResponseOutcome,
};
pub use service::OrderbookApiService;
pub use wire::{ApiClaimSide, ApiMatchKind, ApiOrder, ApiOrderState, ApiSide, ApiSignatureCheck};

#[cfg(test)]
mod tests;
