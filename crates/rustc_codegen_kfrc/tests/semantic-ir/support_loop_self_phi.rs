use core::cmp;
use core::ops::Range;

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Level(u8);

#[derive(Debug, PartialEq)]
pub enum Error {
    OutOfRangeNumber,
}

impl Level {
    #[inline]
    pub fn new(number: u8) -> Result<Level, Error> {
        if number <= 126 {
            Ok(Level(number))
        } else {
            Err(Error::OutOfRangeNumber)
        }
    }
    #[inline]
    pub fn is_ltr(&self) -> bool {
        self.0 % 2 == 0
    }
    #[inline]
    pub fn lower(&mut self, amount: u8) -> Result<(), Error> {
        match self.0.checked_sub(amount) {
            Some(number) => {
                self.0 = number;
                Ok(())
            }
            None => Err(Error::OutOfRangeNumber),
        }
    }
    #[inline]
    pub fn new_lowest_ge_rtl(&self) -> Result<Level, Error> {
        Level::new(self.0 | 1)
    }
}

pub fn reorder_visual(levels: &[Level]) -> Vec<usize> {
    fn next_range(levels: &[Level], mut start_index: usize, max: Level) -> Range<usize> {
        if levels.is_empty() || start_index >= levels.len() {
            return start_index..start_index;
        }
        while let Some(l) = levels.get(start_index) {
            if *l >= max {
                break;
            }
            start_index += 1;
        }
        if levels.get(start_index).is_none() {
            return start_index..start_index;
        }
        let mut end_index = start_index + 1;
        while let Some(l) = levels.get(end_index) {
            if *l < max {
                return start_index..end_index;
            }
            end_index += 1;
        }
        start_index..end_index
    }
    if levels.is_empty() {
        return vec![];
    }
    let (mut min, mut max) = levels
        .iter()
        .fold((levels[0], levels[0]), |(min, max), &l| {
            (cmp::min(min, l), cmp::max(max, l))
        });
    let mut result: Vec<usize> = (0..levels.len()).collect();
    if min == max && min.is_ltr() {
        return result;
    }
    min = min.new_lowest_ge_rtl().expect("Level error");
    while min <= max {
        let mut range = 0..0;
        loop {
            range = next_range(levels, range.end, max);
            result[range.clone()].reverse();
            if range.end >= levels.len() {
                break;
            }
        }
        max.lower(1).expect("Level error");
    }
    result
}

pub fn levels(raw: &[u8]) -> Vec<Level> {
    raw.iter().map(|&n| Level::new(n).unwrap()).collect()
}
