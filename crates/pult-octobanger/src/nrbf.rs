//! .NET's BinaryFormatter stream, read and never run.
//!
//! The format is Microsoft's published `[MS-NRBF]`: a sequence of records, each a tag
//! byte and a body. This reads the records an object graph is made of and nothing
//! else — no method calls, no remoting headers — and it **instantiates nothing**: a
//! class name is a string here, which is the whole difference between reading one of
//! these files and the deserializer that made BinaryFormatter infamous.
//!
//! **The tree is flat.** Every record that carries an object id goes into
//! [`Document::objects`], and a member that holds an object holds a [`Value::Ref`] to
//! it — whether the object was written inline or somewhere later in the stream. So
//! there is one way to follow a member, [`Document::get`], and a reference to an
//! object that has not been read yet is not a special case.
//!
//! Bounded everywhere a file can ask for something: every count is checked against
//! the bytes that are left before anything is allocated for it, and nesting stops at
//! [`MAX_DEPTH`], so a hostile file is an error rather than an allocation or a stack
//! that never ends.

use std::collections::HashMap;

use crate::Error;

/// How deep records may nest inside one another's members.
///
/// The OctoBanger files nest four deep; a controller list inside an event handler
/// inside a binding list is the deepest. Sixty-four is room for any honest file and a
/// stop for a dishonest one.
pub const MAX_DEPTH: usize = 64;

/// One value: a primitive, or a reference to an object in [`Document::objects`].
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Char(char),
    String(String),
    /// A `byte[]`, kept as bytes rather than as a list of numbers.
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Object(Object),
    Ref(i32),
}

/// An instance of a class: its name and its members, in the order the file wrote them.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub class: String,
    pub members: Vec<(String, Value)>,
}

impl Object {
    pub fn member(&self, name: &str) -> Option<&Value> {
        self.members.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

/// A whole stream: the root's id and every object in it.
#[derive(Debug, Default)]
pub struct Document {
    pub root: i32,
    pub objects: HashMap<i32, Value>,
}

impl Document {
    /// Follow a reference, however many times it takes. A value that is not a
    /// reference is itself.
    pub fn resolve<'a>(&'a self, mut value: &'a Value) -> &'a Value {
        // A chain of references is never longer than the objects there are, and a
        // cycle of them is a broken file: stop rather than go round.
        for _ in 0..=self.objects.len() {
            match value {
                Value::Ref(id) => match self.objects.get(id) {
                    Some(next) => value = next,
                    None => return &Value::Null,
                },
                other => return other,
            }
        }
        &Value::Null
    }

    /// The root object.
    pub fn root(&self) -> Option<&Object> {
        match self.objects.get(&self.root).map(|v| self.resolve(v)) {
            Some(Value::Object(object)) => Some(object),
            _ => None,
        }
    }

    /// A member of an object, resolved.
    pub fn get<'a>(&'a self, object: &'a Object, name: &str) -> Option<&'a Value> {
        object.member(name).map(|v| self.resolve(v))
    }
}

/// Read a stream.
pub fn read(bytes: &[u8]) -> Result<Document, Error> {
    let mut reader = Reader { bytes, at: 0, classes: HashMap::new(), document: Document::default() };
    // The header record comes first, and it is what says which object is the root —
    // which is not the first object in the file when an event handler has dragged
    // other things in ahead of it.
    if reader.u8()? != 0 {
        return Err(Error::NotBinaryFormatter);
    }
    reader.document.root = reader.i32()?;
    reader.i32()?; // the header id
    let major = reader.i32()?;
    let minor = reader.i32()?;
    if (major, minor) != (1, 0) {
        return Err(Error::NotBinaryFormatter);
    }
    loop {
        if reader.at >= bytes.len() {
            return Err(Error::Truncated { at: reader.at });
        }
        if reader.record(0)? == Record::End {
            break;
        }
    }
    Ok(reader.document)
}

/// What one record came to, for whoever read it.
#[derive(Debug, PartialEq)]
enum Record {
    Value(Value),
    /// A run of nulls in an array.
    Nulls(usize),
    End,
}

/// How a class's members are typed.
#[derive(Debug, Clone)]
enum Member {
    Primitive(u8),
    /// Anything that is a record of its own.
    Record,
}

#[derive(Debug, Clone)]
struct Class {
    name: String,
    members: Vec<(String, Member)>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    classes: HashMap<i32, Class>,
    document: Document,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).filter(|end| *end <= self.bytes.len());
        let end = end.ok_or(Error::Truncated { at: self.at })?;
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// A count the file asks for, checked against what is left of it: every element
    /// of anything costs at least `each` bytes, so a count that cannot fit is a lie.
    fn count(&mut self, each: usize) -> Result<usize, Error> {
        let at = self.at;
        let n = self.i32()?;
        let n = usize::try_from(n).map_err(|_| Error::Malformed { at, what: "a negative count" })?;
        if n.saturating_mul(each.max(1)) > self.bytes.len() - self.at {
            return Err(Error::Truncated { at });
        }
        Ok(n)
    }

    /// A length-prefixed string: seven bits of length per byte, then UTF-8.
    fn string(&mut self) -> Result<String, Error> {
        let at = self.at;
        let mut length = 0usize;
        for shift in (0..35).step_by(7) {
            let byte = self.u8()?;
            length |= ((byte & 0x7f) as usize) << shift;
            if byte & 0x80 == 0 {
                let text = self.take(length)?;
                return String::from_utf8(text.to_vec()).map_err(|_| Error::Malformed { at, what: "a string that is not UTF-8" });
            }
        }
        Err(Error::Malformed { at, what: "a string length longer than five bytes" })
    }

    fn primitive(&mut self, kind: u8) -> Result<Value, Error> {
        let at = self.at;
        Ok(match kind {
            1 => Value::Bool(self.u8()? != 0),
            2 => Value::UInt(self.u8()? as u64),
            3 => {
                // One character, UTF-8 encoded: as long as its first byte says.
                let first = self.u8()?;
                let length = match first {
                    0x00..=0x7f => 1,
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    _ => 4,
                };
                let mut buffer = vec![first];
                buffer.extend_from_slice(self.take(length - 1)?);
                let text = String::from_utf8(buffer).map_err(|_| Error::Malformed { at, what: "a char that is not UTF-8" })?;
                Value::Char(text.chars().next().unwrap_or('\0'))
            }
            5 => Value::String(self.string()?), // Decimal, which the format writes as text
            6 => Value::Float(f64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            7 => Value::Int(i16::from_le_bytes(self.take(2)?.try_into().unwrap()) as i64),
            8 => Value::Int(self.i32()? as i64),
            9 | 12 | 13 => Value::Int(i64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            10 => Value::Int(self.u8()? as i8 as i64),
            11 => Value::Float(f32::from_le_bytes(self.take(4)?.try_into().unwrap()) as f64),
            14 => Value::UInt(u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as u64),
            15 => Value::UInt(u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as u64),
            16 => Value::UInt(u64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            17 => Value::Null,
            18 => Value::String(self.string()?),
            other => return Err(Error::Malformed { at, what: primitive_name(other) }),
        })
    }

    /// The least a primitive of this kind can take, for checking an array's count.
    fn primitive_size(kind: u8) -> usize {
        match kind {
            6 | 9 | 12 | 13 | 16 => 8,
            8 | 11 | 15 => 4,
            7 | 14 => 2,
            _ => 1,
        }
    }

    /// The member type list of a class record, and the extra information each type
    /// carries.
    fn member_types(&mut self, names: Vec<String>) -> Result<Vec<(String, Member)>, Error> {
        let mut kinds = Vec::with_capacity(names.len());
        for _ in &names {
            kinds.push(self.u8()?);
        }
        let mut members = Vec::with_capacity(names.len());
        for (name, kind) in names.into_iter().zip(kinds) {
            let member = match kind {
                0 => Member::Primitive(self.u8()?),
                7 => {
                    self.u8()?;
                    Member::Record
                }
                3 => {
                    self.string()?;
                    Member::Record
                }
                4 => {
                    self.string()?;
                    self.i32()?;
                    Member::Record
                }
                1 | 2 | 5 | 6 => Member::Record,
                _ => return Err(Error::Malformed { at: self.at, what: "a member type this format does not have" }),
            };
            members.push((name, member));
        }
        Ok(members)
    }

    fn class_info(&mut self) -> Result<(i32, String, Vec<String>), Error> {
        let id = self.i32()?;
        let name = self.string()?;
        let count = self.count(1)?;
        let mut names = Vec::with_capacity(count);
        for _ in 0..count {
            names.push(self.string()?);
        }
        Ok((id, name, names))
    }

    /// The members of an instance, in the order its class says.
    fn instance(&mut self, id: i32, class: Class, depth: usize) -> Result<Value, Error> {
        let mut members = Vec::with_capacity(class.members.len());
        for (name, member) in &class.members {
            let value = match member {
                Member::Primitive(kind) => self.primitive(*kind)?,
                Member::Record => match self.record(depth + 1)? {
                    Record::Value(value) => value,
                    Record::Nulls(_) => Value::Null,
                    Record::End => return Err(Error::Truncated { at: self.at }),
                },
            };
            members.push((name.clone(), value));
        }
        self.document.objects.insert(id, Value::Object(Object { class: class.name, members }));
        Ok(Value::Ref(id))
    }

    /// The elements of an array of records, with runs of nulls expanded.
    fn elements(&mut self, count: usize, depth: usize) -> Result<Vec<Value>, Error> {
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            match self.record(depth + 1)? {
                Record::Value(value) => out.push(value),
                Record::Nulls(n) => {
                    if out.len() + n > count {
                        return Err(Error::Malformed { at: self.at, what: "more nulls than the array has room for" });
                    }
                    out.extend(std::iter::repeat_n(Value::Null, n));
                }
                Record::End => return Err(Error::Truncated { at: self.at }),
            }
        }
        Ok(out)
    }

    fn store(&mut self, id: i32, value: Value) -> Record {
        self.document.objects.insert(id, value);
        Record::Value(Value::Ref(id))
    }

    fn record(&mut self, depth: usize) -> Result<Record, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::TooDeep { at: self.at });
        }
        let at = self.at;
        let tag = self.u8()?;
        Ok(match tag {
            // ClassWithId: another instance of a class already described.
            1 => {
                let id = self.i32()?;
                let metadata = self.i32()?;
                let class = self.classes.get(&metadata).cloned().ok_or(Error::Malformed { at, what: "an instance of a class never described" })?;
                Record::Value(self.instance(id, class, depth)?)
            }
            // SystemClassWithMembersAndTypes, ClassWithMembersAndTypes.
            4 | 5 => {
                let (id, name, names) = self.class_info()?;
                let members = self.member_types(names)?;
                if tag == 5 {
                    self.i32()?; // the library it is in
                }
                let class = Class { name, members };
                self.classes.insert(id, class.clone());
                Record::Value(self.instance(id, class, depth)?)
            }
            2 | 3 => return Err(Error::Unsupported { at, record: "a class written without its member types" }),
            // BinaryObjectString.
            6 => {
                let id = self.i32()?;
                let text = self.string()?;
                self.store(id, Value::String(text))
            }
            // BinaryArray: any rank, any element type.
            7 => {
                let id = self.i32()?;
                let shape = self.u8()?;
                let rank = self.count(4)?;
                let mut length = 1usize;
                for _ in 0..rank {
                    length = length.saturating_mul(self.count(1)?);
                }
                if matches!(shape, 3..=5) {
                    for _ in 0..rank {
                        self.i32()?;
                    }
                }
                let kind = self.u8()?;
                let primitive = match kind {
                    0 | 7 => Some(self.u8()?),
                    3 => {
                        self.string()?;
                        None
                    }
                    4 => {
                        self.string()?;
                        self.i32()?;
                        None
                    }
                    _ => None,
                };
                let value = match (kind, primitive) {
                    (0, Some(p)) => {
                        if length.saturating_mul(Self::primitive_size(p)) > self.bytes.len() - self.at {
                            return Err(Error::Truncated { at });
                        }
                        Value::Array((0..length).map(|_| self.primitive(p)).collect::<Result<_, _>>()?)
                    }
                    _ => {
                        if length > self.bytes.len() - self.at {
                            return Err(Error::Truncated { at });
                        }
                        Value::Array(self.elements(length, depth)?)
                    }
                };
                self.store(id, value)
            }
            // MemberPrimitiveTyped: a primitive where an object could have been.
            8 => {
                let kind = self.u8()?;
                Record::Value(self.primitive(kind)?)
            }
            // MemberReference.
            9 => Record::Value(Value::Ref(self.i32()?)),
            10 => Record::Value(Value::Null),
            11 => Record::End,
            // BinaryLibrary: names an assembly, and is followed by the record it was
            // written for.
            12 => {
                self.i32()?;
                self.string()?;
                return self.record(depth);
            }
            13 => Record::Nulls(self.u8()? as usize),
            14 => Record::Nulls(self.count(0)?),
            // ArraySinglePrimitive.
            15 => {
                let id = self.i32()?;
                let length = self.i32()?;
                let kind = self.u8()?;
                let length = usize::try_from(length).map_err(|_| Error::Malformed { at, what: "a negative count" })?;
                if length.saturating_mul(Self::primitive_size(kind)) > self.bytes.len() - self.at {
                    return Err(Error::Truncated { at });
                }
                let value = if kind == 2 {
                    Value::Bytes(self.take(length)?.to_vec())
                } else {
                    Value::Array((0..length).map(|_| self.primitive(kind)).collect::<Result<_, _>>()?)
                };
                self.store(id, value)
            }
            // ArraySingleObject, ArraySingleString.
            16 | 17 => {
                let id = self.i32()?;
                let length = self.count(1)?;
                let elements = self.elements(length, depth)?;
                self.store(id, Value::Array(elements))
            }
            21 | 22 => return Err(Error::Unsupported { at, record: "a remoting call" }),
            _ => return Err(Error::Malformed { at, what: "a record type this format does not have" }),
        })
    }
}

fn primitive_name(kind: u8) -> &'static str {
    match kind {
        4 => "a primitive of the unused kind 4",
        _ => "a primitive type this format does not have",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream holding one class with an int, a string and a byte array.
    fn small() -> Vec<u8> {
        let mut b = vec![0];
        for v in [1i32, -1, 1, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.push(12);
        b.extend_from_slice(&2i32.to_le_bytes());
        b.push(3);
        b.extend_from_slice(b"Lib");
        b.push(5);
        b.extend_from_slice(&1i32.to_le_bytes());
        b.push(5);
        b.extend_from_slice(b"Thing");
        b.extend_from_slice(&3i32.to_le_bytes());
        for name in ["n", "s", "data"] {
            b.push(name.len() as u8);
            b.extend_from_slice(name.as_bytes());
        }
        b.extend_from_slice(&[0, 1, 7]); // primitive, string, primitive array
        b.push(8); // int32
        b.push(2); // byte
        b.extend_from_slice(&2i32.to_le_bytes()); // the library
        b.extend_from_slice(&42i32.to_le_bytes());
        b.push(6);
        b.extend_from_slice(&3i32.to_le_bytes());
        b.push(2);
        b.extend_from_slice(b"hi");
        b.push(9);
        b.extend_from_slice(&4i32.to_le_bytes());
        b.push(15);
        b.extend_from_slice(&4i32.to_le_bytes());
        b.extend_from_slice(&3i32.to_le_bytes());
        b.push(2);
        b.extend_from_slice(&[7, 8, 9]);
        b.push(11);
        b
    }

    #[test]
    fn a_class_its_members_and_a_forward_reference() {
        let document = read(&small()).unwrap();
        let root = document.root().unwrap();
        assert_eq!(root.class, "Thing");
        assert_eq!(document.get(root, "n"), Some(&Value::Int(42)));
        assert_eq!(document.get(root, "s"), Some(&Value::String("hi".into())));
        assert_eq!(document.get(root, "data"), Some(&Value::Bytes(vec![7, 8, 9])), "read after the object that points at it");
    }

    #[test]
    fn every_prefix_of_a_stream_is_an_error_and_never_a_panic() {
        let whole = small();
        for end in 0..whole.len() {
            assert!(read(&whole[..end]).is_err(), "{end} bytes read as a whole stream");
        }
    }

    #[test]
    fn a_count_larger_than_the_file_is_refused_before_anything_is_allocated() {
        let mut b = small();
        // The byte array's length, rewritten to two billion.
        let at = b.len() - 1 - 3 - 1 - 4;
        b[at..at + 4].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(matches!(read(&b), Err(Error::Truncated { .. })));
    }

    #[test]
    fn something_else_entirely_is_named_as_such() {
        assert!(matches!(read(b"<?xml version=\"1.0\"?>"), Err(Error::NotBinaryFormatter)));
    }
}
