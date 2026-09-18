//! Bounded protobuf field access for build-specific protocol profiles.
const MAX_PACKET: usize = 4 * 1024 * 1024;
const MAX_FIELDS: usize = 65536;

#[derive(Debug, Clone, PartialEq)]
pub enum Value<'a> {
    Varint(u64),
    Fixed32(u32),
    Fixed64,
    Bytes(&'a [u8]),
}
#[derive(Debug, Clone, PartialEq)]
pub struct Field<'a> {
    pub number: u32,
    pub value: Value<'a>,
}

fn varint(input: &[u8], offset: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *input.get(*offset).ok_or("Truncated varint")?;
        *offset += 1;
        if shift == 63 && byte > 1 {
            return Err("Varint overflow".into());
        }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Ok(value);
        }
    }
    Err("Varint overflow".into())
}

pub fn fields(input: &[u8]) -> Result<Vec<Field<'_>>, String> {
    if input.len() > MAX_PACKET {
        return Err("Packet exceeds size limit".into());
    }
    let mut offset = 0;
    let mut out = Vec::new();
    while offset < input.len() {
        if out.len() >= MAX_FIELDS {
            return Err("Too many protobuf fields".into());
        }
        let tag = varint(input, &mut offset)?;
        let number = u32::try_from(tag >> 3).map_err(|_| "Invalid field number")?;
        if number == 0 || number > 0x1fff_ffff {
            return Err("Invalid field number".into());
        }
        let value = match tag & 7 {
            0 => Value::Varint(varint(input, &mut offset)?),
            wire @ (1 | 2 | 5) => {
                let len = match wire {
                    1 => 8,
                    5 => 4,
                    _ => usize::try_from(varint(input, &mut offset)?)
                        .map_err(|_| "Length overflow")?,
                };
                let end = offset.checked_add(len).ok_or("Length overflow")?;
                let data = input.get(offset..end).ok_or("Truncated protobuf field")?;
                offset = end;
                match wire {
                    1 => Value::Fixed64,
                    5 => Value::Fixed32(u32::from_le_bytes(data.try_into().unwrap())),
                    _ => Value::Bytes(data),
                }
            }
            _ => return Err("Unsupported protobuf wire type".into()),
        };
        out.push(Field { number, value });
    }
    Ok(out)
}

pub fn messages<'a>(input: &'a [u8], path: &[u32]) -> Result<Vec<&'a [u8]>, String> {
    if path.len() > 12 {
        return Err("Path exceeds depth limit".into());
    }
    let mut current = vec![input];
    for &number in path {
        let mut next = Vec::new();
        for message in current {
            for field in fields(message)? {
                if field.number == number {
                    match field.value {
                        Value::Bytes(bytes) => next.push(bytes),
                        _ => return Err("Expected nested message".into()),
                    }
                    if next.len() > 4096 {
                        return Err("Too many nested messages".into());
                    }
                }
            }
        }
        current = next;
    }
    Ok(current)
}

pub fn scalar(input: &[u8], path: &[u32]) -> Result<Option<u64>, String> {
    let (&last, parents) = path.split_last().ok_or("Empty scalar path")?;
    let mut found = None;
    for message in messages(input, parents)? {
        for field in fields(message)? {
            if field.number == last {
                if found.is_some() {
                    return Err("Ambiguous scalar value".into());
                }
                match field.value {
                    Value::Varint(value) => found = Some(value),
                    _ => return Err("Expected integer".into()),
                }
            }
        }
    }
    Ok(found)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}
impl Position {
    pub fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

pub fn vector(input: &[u8], axes: [u32; 3]) -> Result<Position, String> {
    let mut values = [0f32; 3]; // proto3 omits zero components.
    let mut seen = [false; 3];
    for field in fields(input)? {
        if let Some(i) = axes.iter().position(|&axis| axis == field.number) {
            if seen[i] {
                return Err("Duplicate coordinate component".into());
            }
            match field.value {
                Value::Fixed32(bits) => values[i] = f32::from_bits(bits),
                _ => return Err("Expected float32 coordinate".into()),
            }
            seen[i] = true;
        } else {
            // A different layout must not become an apparently valid zero vector.
            return Err(format!(
                "Unexpected coordinate field {}; update the protocol profile",
                field.number
            ));
        }
    }
    let position = Position {
        x: values[0],
        y: values[1],
        z: values[2],
    };
    if !position.finite() {
        return Err("Non-finite coordinate".into());
    }
    Ok(position)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protobuf_rejects_malformed_and_nonfinite() {
        for bytes in [
            &[0][..],
            &[10, 255][..],
            &[13, 0][..],
            &[8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 2][..],
        ] {
            assert!(fields(bytes).is_err());
        }
        let mut bytes = vec![13];
        bytes.extend_from_slice(&f32::NAN.to_le_bytes());
        assert!(vector(&bytes, [1, 2, 3]).is_err());
    }
    #[test]
    fn vector_zero_defaults_and_nested_paths() {
        let mut bytes = vec![21];
        bytes.extend_from_slice(&(-0.0f32).to_le_bytes());
        let p = vector(&bytes, [1, 2, 3]).unwrap();
        assert_eq!(p.x, 0.);
        assert_eq!(p.y.to_bits(), (-0.0f32).to_bits());
        assert_eq!(scalar(&[10, 2, 8, 42], &[1, 1]).unwrap(), Some(42));
        assert!(scalar(&[8, 1, 8, 2], &[1]).is_err());
    }

    #[test]
    fn changed_vector_layouts_cannot_silently_become_zero_positions() {
        assert_eq!(
            vector(&[], [1, 2, 3]).unwrap(),
            Position {
                x: 0.,
                y: 0.,
                z: 0.
            }
        );
        let mut shuffled = vec![37]; // float32 field 4 instead of a known axis.
        shuffled.extend(12.5_f32.to_le_bytes());
        assert!(
            vector(&shuffled, [1, 2, 3])
                .unwrap_err()
                .contains("Unexpected coordinate field 4")
        );
        assert_eq!(vector(&shuffled, [4, 5, 6]).unwrap().x, 12.5);
        let mut mixed = vec![13];
        mixed.extend(1_f32.to_le_bytes());
        mixed.extend(shuffled);
        assert!(vector(&mixed, [1, 2, 3]).is_err());
        assert!(vector(&[8, 1], [1, 2, 3]).is_err());
    }
}
