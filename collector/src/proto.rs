// Minimal protobuf wire-format reader (no schema). Enough to walk nested
// length-delimited messages and pull out varints and strings by field path.

#[derive(Debug, Clone)]
pub enum Wire<'a> {
    Varint(u64),
    Fixed64,
    Fixed32,
    Len(&'a [u8]),
}

fn varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut r: u64 = 0;
    let mut s = 0;
    loop {
        let c = *b.get(*i)?;
        *i += 1;
        if s < 64 {
            r |= u64::from(c & 0x7f) << s;
        }
        s += 7;
        if c < 0x80 {
            return Some(r);
        }
        if s > 70 {
            return None;
        }
    }
}

/// Decodes one message level. Returns `None` if the bytes are not valid wire format.
pub fn fields(b: &[u8]) -> Option<Vec<(u32, Wire<'_>)>> {
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let key = varint(b, &mut i)?;
        let f = (key >> 3) as u32;
        if f == 0 {
            return None;
        }
        match key & 7 {
            0 => out.push((f, Wire::Varint(varint(b, &mut i)?))),
            1 => {
                i = i.checked_add(8)?;
                if i > b.len() {
                    return None;
                }
                out.push((f, Wire::Fixed64));
            }
            2 => {
                let l = varint(b, &mut i)? as usize;
                let end = i.checked_add(l)?;
                if end > b.len() {
                    return None;
                }
                out.push((f, Wire::Len(&b[i..end])));
                i = end;
            }
            5 => {
                i = i.checked_add(4)?;
                if i > b.len() {
                    return None;
                }
                out.push((f, Wire::Fixed32));
            }
            _ => return None,
        }
    }
    Some(out)
}

/// First occurrence of `field` as a nested message.
pub fn sub<'a>(fs: &[(u32, Wire<'a>)], field: u32) -> Option<Vec<(u32, Wire<'a>)>> {
    fs.iter().find_map(|(f, w)| match w {
        Wire::Len(b) if *f == field => fields(b),
        _ => None,
    })
}

pub fn int(fs: &[(u32, Wire<'_>)], field: u32) -> Option<u64> {
    fs.iter().find_map(|(f, w)| match w {
        Wire::Varint(v) if *f == field => Some(*v),
        _ => None,
    })
}

pub fn str<'a>(fs: &[(u32, Wire<'a>)], field: u32) -> Option<&'a str> {
    fs.iter().find_map(|(f, w)| match w {
        Wire::Len(b) if *f == field => std::str::from_utf8(b).ok(),
        _ => None,
    })
}
