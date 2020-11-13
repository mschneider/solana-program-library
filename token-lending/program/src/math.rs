//! Math for preserving precision

#![allow(clippy::assign_op_pattern)]
#![allow(clippy::ptr_offset_with_cast)]

use std::fmt;
use uint::construct_uint;

// U256 with 256 bits consisting of 4 x 64-bit words
construct_uint! {
    pub struct U256(4);
}

/// Decimal value precise to 18 digits
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Decimal(U256);

const SCALE: usize = 18;

impl Decimal {
    fn scaler() -> U256 {
        U256::exp10(SCALE)
    }

    /// Create scaled decimal from value and scale
    pub fn new(val: u64, scale: usize) -> Self {
        assert!(scale <= SCALE);
        Self(Self::scaler() / U256::exp10(scale) * U256::from(val))
    }

    /// Return raw scaled value
    pub fn to_scaled_val(&self) -> u128 {
        self.0.as_u128()
    }

    /// Create decimal from scaled value
    pub fn from_scaled_val(scaled_val: u128) -> Self {
        Self(U256::from(scaled_val))
    }

    /// Round scaled decimal to u64
    pub fn round_u64(&self) -> u64 {
        (self.0 / Self::scaler()).as_u64()
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut scaled_val = self.0.to_string();
        if scaled_val.len() <= SCALE {
            scaled_val.insert_str(0, &vec!["0"; SCALE - scaled_val.len()].join(""));
            scaled_val.insert_str(0, "0.");
        } else {
            scaled_val.insert(scaled_val.len() - SCALE, '.');
        }
        f.write_str(&scaled_val)
    }
}

impl From<u64> for Decimal {
    fn from(val: u64) -> Self {
        Self(Self::scaler() * U256::from(val))
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
        Self(Self::scaler() * self.0 / rhs.0)
    }
}

impl std::ops::Mul for Decimal {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0 / Self::scaler())
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
