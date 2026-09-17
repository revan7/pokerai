use crate::{error::AppError, service::{Op, Service}};
use proto::*;
use tauri::{ipc::Channel, State};

macro_rules! command {
    ($name:ident, $ret:ty, $op:expr $(, $arg:ident : $ty:ty)*) => {
        #[tauri::command(rename_all = "snake_case")]
        pub async fn $name(service: State<'_, Service>, $($arg: $ty),*) -> Result<$ret, AppError> {
            service.request($op).await
        }
    };
}

command!(set_game_config, GameConfig, Op::Config(config), config: GameConfig);
command!(begin_hand, HandState, Op::Begin(begin), begin: BeginHand);
command!(set_hero_cards, HandState, Op::Hero(cards), cards: [Card; 2]);
command!(apply_action, HandState, Op::Action(action), action: Action);
command!(set_board, HandState, Op::Board(board), board: Vec<Card>);
command!(undo, HandState, Op::Undo);
command!(recommend, DecisionIdentity, Op::Recommend(on_event), on_event: Channel<RecommendationEvent>);
command!(cancel, (), Op::Cancel(decision_id), decision_id: u64);
command!(finish_hand, (), Op::Finish);
command!(abandon_hand, (), Op::Abandon);
command!(presolver_status, serde_json::Value, Op::PresolverStatus);
command!(presolver_pause, (), Op::Pause);
command!(presolver_resume, (), Op::Resume);

#[tauri::command(rename_all = "snake_case")]
pub async fn set_seat_tag(seat: Seat, tag: SeatTag) -> Result<(), AppError> {
    let _ = (seat, tag);
    Err(AppError::Unsupported {
        reason: UnsupportedReason::FormatUnsupported {
            detail: "seat tags are unsupported in phase 1".into(),
        },
    })
}
