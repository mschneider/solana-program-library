//! Math for preserving precision

use std::fmt;

/// Decimal value precise to 9 digits
/// TODO: use bigint
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Decimal(u128);

const SCALE: usize = 9;
const SCALER: u128 = 1_000_000_000;

impl Decimal {
    /// Create scaled decimal from unscaled value
    pub const fn from_val(val: u128) -> Self {
        Self(SCALER * val)
    }

    /// Create scaled decimal from value and scale
    pub fn new(val: u64, scale: u32) -> Self {
        Self(SCALER / 10u128.pow(scale) * val as u128)
    }

    /// Return raw scaled value
    pub fn scaled_val(&self) -> u128 {
        self.0
    }

    /// Create decimal from scaled value
    pub fn from_scaled_val(scaled_val: u128) -> Self {
        Self(scaled_val)
    }

    /// Round scaled decimal to u64
    pub fn round_u64(&self) -> u64 {
        (self.0 / SCALER) as u64
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut scaled_val = self.0.to_string();
        if scaled_val.len() <= SCALE {
            scaled_val.insert_str(0, &vec!["0"; SCALE - scaled_val.len()].join(""));
            scaled_val.insert_str(0, "0.");
        } else {
            scaled_val.insert_str(scaled_val.len() - SCALE, ".");
        }
        f.write_str(&scaled_val)
    }
}

impl From<u64> for Decimal {
    fn from(val: u64) -> Self {
        Self(SCALER * val as u128)
    }
}

impl std::ops::Add for Decimal {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Decimal {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::Div for Decimal {
    type Output = Self;
    fn div(self, rhs: Self) -> Self::Output {
        Self(SCALER * self.0 / rhs.0)
    }
}

impl std::ops::Mul for Decimal {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0 / SCALER)
    }
}

impl std::ops::AddAssign for Decimal {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl std::ops::SubAssign for Decimal {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl std::ops::DivAssign for Decimal {
    fn div_assign(&mut self, rhs: Self) {
        *self = *self / rhs;
    }
}

impl std::ops::MulAssign for Decimal {
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}
