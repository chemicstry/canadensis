//! A bit array type

use crate::{ReadCursor, WriteCursor};
use core::cmp::Ordering;
use core::fmt;

/// An array of bits in a format compatible with UAVCAN serialization
///
/// Because the const generics feature is incomplete, the integer generic parameter is a number
/// of bytes (= 8 bits), not a number of bits. The functions still use bit indexes.
#[derive(Clone)]
pub struct BitArray<const BYTES: usize> {
    bytes: [u8; BYTES],
    bit_length: usize,
}

impl<const BYTES: usize> BitArray<BYTES> {
    /// Creates a bit set with all bits set to zero
    pub fn new(bit_length: usize) -> Self {
        assert!(bit_length <= BYTES * 8);
        BitArray {
            bytes: [0; BYTES],
            bit_length,
        }
    }

    /// Returns the number of bits in this array
    pub fn len(&self) -> usize {
        self.bit_length
    }
    /// Returns true if this array is empty
    pub fn is_empty(&self) -> bool {
        self.bit_length == 0
    }

    /// Returns the value of a bit at the provided bit index
    pub fn get(&self, bit_index: usize) -> bool {
        let (byte_index, bit_in_byte) = self.split_index(bit_index);
        let byte = self.bytes[byte_index];
        let bit = (byte >> bit_in_byte) & 1;
        bit == 1
    }

    /// Sets the value of a bit at the provided bit index
    pub fn set(&mut self, bit_index: usize, value: bool) {
        let (byte_index, bit_in_byte) = self.split_index(bit_index);
        let mask = 1 << bit_in_byte;
        let byte = &mut self.bytes[byte_index];
        if value {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }

    /// Sets all bits in this array to the specified value
    pub fn fill(&mut self, value: bool) {
        if value {
            self.bytes.fill(0xff);
        } else {
            self.bytes.fill(0x00);
        }
    }

    /// Serializes this bit set (not including the length)
    ///
    /// Note: This type doesn't implement DataType, Serialize, or Deserialize because it is not a
    /// composite type and has an alignment of only 1 bit.
    pub fn serialize(&self, cursor: &mut WriteCursor<'_>) {
        if self.bit_length == BYTES * 8 && cursor.is_aligned_to_8_bits() {
            cursor.write_aligned_bytes(&self.bytes);
        } else {
            for bit_index in 0..self.bit_length {
                cursor.write_bool(self.get(bit_index));
            }
        }
    }

    /// Deserializes this bit set (not including the length). self.bit_length must be set
    /// before this function is called.
    pub fn deserialize_in_place(&mut self, cursor: &mut ReadCursor<'_>) {
        if self.bit_length % 8 == 0 && cursor.is_aligned_to_8_bits() {
            self.bytes.fill_with(|| cursor.read_aligned_u8());
        } else {
            for i in 0..self.bit_length {
                self.set(i, cursor.read_bool());
            }
        }
    }

    /// Deserializes a bit set
    ///
    /// `bit_length` is the number of bits that should be read from the cursor.
    pub fn deserialize(bit_length: usize, cursor: &mut ReadCursor<'_>) -> Self {
        let mut set = BitArray::new(bit_length);
        set.deserialize_in_place(cursor);
        set
    }

    /// Returns an iterator over the bits in this array
    pub fn iter(&self) -> Iter<'_, BYTES> {
        Iter {
            array: self,
            next_index: 0,
        }
    }

    /// Splits a bit index into a byte index and an index of the bit within the byte
    ///
    /// # Panics
    ///
    /// This function panics if bit_index is greater than or equal to `self.len()`.
    fn split_index(&self, bit_index: usize) -> (usize, u8) {
        assert!(bit_index < self.bit_length);
        // The UAVCAN serialization makes this simple
        let byte = bit_index / 8;
        let bit_in_byte = (bit_index % 8) as u8;
        (byte, bit_in_byte)
    }
}

impl<const BYTES: usize> fmt::Debug for BitArray<BYTES> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list_debug = f.debug_list();
        for i in 0..self.len() {
            list_debug.entry(&self.get(i));
        }
        list_debug.finish()
    }
}

impl<const BYTES: usize> Default for BitArray<BYTES> {
    /// Creates a new array with a length of zero
    fn default() -> Self {
        BitArray {
            bytes: [0u8; BYTES],
            bit_length: 0,
        }
    }
}

/// An iterator over values in a bit array
pub struct Iter<'a, const BYTES: usize> {
    array: &'a BitArray<BYTES>,
    next_index: usize,
}

impl<const BYTES: usize> Iterator for Iter<'_, BYTES> {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index == self.array.len() {
            None
        } else {
            let value = self.array.get(self.next_index);
            self.next_index += 1;
            Some(value)
        }
    }
}

impl<'a, const BYTES: usize> IntoIterator for &'a BitArray<BYTES> {
    type Item = bool;
    type IntoIter = Iter<'a, BYTES>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over values in a bit array
pub struct IntoIter<const BYTES: usize> {
    array: BitArray<BYTES>,
    next_index: usize,
}

impl<const BYTES: usize> Iterator for IntoIter<BYTES> {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index == self.array.len() {
            None
        } else {
            let value = self.array.get(self.next_index);
            self.next_index += 1;
            Some(value)
        }
    }
}

impl<const BYTES: usize> IntoIterator for BitArray<BYTES> {
    type Item = bool;
    type IntoIter = IntoIter<BYTES>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            array: self,
            next_index: 0,
        }
    }
}

impl<const LBYTES: usize, const RBYTES: usize> PartialEq<BitArray<RBYTES>> for BitArray<LBYTES> {
    fn eq(&self, other: &BitArray<RBYTES>) -> bool {
        if self.len() == other.len() {
            // This may be slow, but it's good enough for now.
            for (lbit, rbit) in self.iter().zip(other.iter()) {
                if lbit != rbit {
                    return false;
                }
            }
            true
        } else {
            false
        }
    }
}

impl<const LBYTES: usize> Eq for BitArray<LBYTES> {}

impl<const LBYTES: usize, const RBYTES: usize> PartialOrd<BitArray<RBYTES>> for BitArray<LBYTES> {
    fn partial_cmp(&self, other: &BitArray<RBYTES>) -> Option<Ordering> {
        Some(compare(self, other))
    }
}

impl<const LBYTES: usize> Ord for BitArray<LBYTES> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare(self, other)
    }
}

fn compare<const LBYTES: usize, const RBYTES: usize>(
    lhs: &BitArray<LBYTES>,
    rhs: &BitArray<RBYTES>,
) -> Ordering {
    lhs.len().cmp(&rhs.len()).then_with(|| {
        // lhs and rhs have the same length
        // Compare bits
        for (lbit, rbit) in lhs.iter().zip(rhs.iter()) {
            match (lbit, rbit) {
                (true, false) => return Ordering::Greater,
                (false, true) => return Ordering::Less,
                _ => {}
            }
        }
        // All bits equal
        Ordering::Equal
    })
}
