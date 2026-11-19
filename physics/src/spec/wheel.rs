use serde::{Deserialize, Serialize};

/// Four wheels, in a fixed order. Index into `[WheelSpec; 4]` / `[WheelState; 4]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Wheel {
    FrontLeft = 0,
    FrontRight = 1,
    RearLeft = 2,
    RearRight = 3,
}

impl Wheel {
    pub const ALL: [Wheel; 4] = [
        Wheel::FrontLeft,
        Wheel::FrontRight,
        Wheel::RearLeft,
        Wheel::RearRight,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn is_front(self) -> bool {
        matches!(self, Wheel::FrontLeft | Wheel::FrontRight)
    }

    pub const fn is_left(self) -> bool {
        matches!(self, Wheel::FrontLeft | Wheel::RearLeft)
    }
}
