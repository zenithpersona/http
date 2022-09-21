#![feature(try_blocks)]
#![feature(let_else)]

use std::error;
use std::fmt;
use std::io;
use std::mem;
use std::net;
use std::result;

const LOCAL_HOST: &'static str = "127.0.0.1";

type Target = String;
type Status = String;
type Body<'a> = &'a str;

#[derive(Debug)]
pub enum Error {
    AddrInUse,
    Malformed,
}

type Result<T> = result::Result<T, Error>;

#[derive(Clone, Copy, Debug)]
#[repr(u16)]
enum Code {
    Success = 200,
}

impl From<Code> for u16 {
    fn from(code: Code) -> Self {
        unsafe { mem::transmute(code) }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::AddrInUse => Self::AddrInUse,
            _ => panic!("unknown io error mapping"),
        }
    }
}

impl error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{:?}", self))
    }
}

#[derive(Debug, Clone, Copy)]
struct Version {
    major: u8,
    minor: u8,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[derive(Debug, Clone, Copy)]
enum Method {
    Get,
    Head,
    Post,
    Put,
    Delete,
    Connect,
    Options,
    Trace,
    Patch,
}

impl TryFrom<&'_ str> for Method {
    type Error = Error;

    fn try_from(data: &'_ str) -> Result<Self> {
        Ok(match data.to_ascii_lowercase().as_str() {
            "get" => Self::Get,
            "head" => Self::Head,
            "post" => Self::Post,
            "put" => Self::Put,
            "delete" => Self::Delete,
            "connect" => Self::Connect,
            "options" => Self::Options,
            "trace" => Self::Trace,
            "patch" => Self::Patch,
            _ => Err(Error::Malformed)?
        })
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{:?}", &self).to_ascii_uppercase())
    }
}

enum Message {
    Request {
        method: Method,
        target: Target,
        version: Version,
        frames: Vec<Frame>,
    },
    Response {
        version: Version,
        code: Code,
        frames: Vec<Frame>,
    },
}

impl Message {
    pub fn into_bytes(self) -> Vec<u8> {
        use Message::*;
        use Frame::*;
        
        let mut info = String::new();

        let begin = match self {
            Request { method, ref target, version, .. } => {
                format!("{} {} HTTP/{}\r\n", method, target, version)
            }
            Response { version, code, .. } => {
                format!("HTTP/{} {} {}\r\n", version, u16::from(code), code)
            }
        };

        info += &begin;
        
        let frames = match self {
            Request { frames, .. } => { frames }
            Response { frames, .. } => { frames }
        };

        let mut payload = vec![];

        for frame in frames {
            match frame {
                Headers { headers } => {
                    for header in headers {
                        let header = format!("{}: {}\r\n", header.name, header.value);
                        info += &header;
                    }
                }
                Data { payload: data } => {
                    payload.extend(data);
                }
            }
        }

        let mut response = vec![];

        response.extend(info.into_bytes());
        response.extend("\r\n".bytes());
        response.extend(payload);

        response
    }

    pub fn parse(buffer: &[u8]) -> Result<Self> {
        let Ok(mut buffer) = String::from_utf8(buffer.iter().cloned().collect()) else {
            return Err(Error::Malformed);
        };

        if buffer.len() == 0 {
            return Err(Error::Malformed);
        }

        let mut cursor = buffer.lines();

        let message: Option<Message> = try {
            let status_line = cursor.next()?;

            let (method, target, version) = Self::parse_status_line(&status_line)?;

            let mut headers = String::new();

            while let Some(h) = cursor.next() {
                if h.len() == 0 {
                    break;
                }

                headers += h;
                headers += "\r\n";
            }

            let headers = Self::parse_headers(&headers)?;

            let mut content_length = None;

            for header in &headers {
                if header.name == "Content-Length" {
                    content_length = Some(header.value.parse::<usize>().ok()?);
                    break;
                }
            }

            let headers = Frame::Headers { headers };

            let mut frames = vec![];

            frames.push(headers);

            if let Some(l) = content_length {
                let rest = cursor.collect::<String>();

                buffer = rest.chars().skip(l).collect::<String>();

                cursor = buffer.lines();

                let payload = rest.chars().take(l).collect::<String>().into_bytes();

                let data = Frame::Data { payload };

                frames.push(data);
            }

            Message::Request {
                method,
                target,
                version,
                frames,
            }
        };

        message.ok_or(Error::Malformed)
    }

    fn parse_status_line(info: &'_ str) -> Option<(Method, Target, Version)> {
        let data = info.split_whitespace().collect::<Vec<_>>();

        let method = (*data.get(0)?).try_into().ok()?;

        let target = data.get(1)?.to_string();

        let major = data
            .get(2)?
            .split("HTTP/")
            .collect::<Vec<_>>()
            .get(1)?
            .split(".")
            .collect::<Vec<_>>()
            .get(0)?
            .parse::<u8>()
            .unwrap();

        let minor = data
            .get(2)?
            .split("HTTP/")
            .collect::<Vec<_>>()
            .get(1)?
            .split(".")
            .collect::<Vec<_>>()
            .get(1)?
            .parse::<u8>()
            .unwrap();

        let version = Version { major, minor };

        Some((method, target, version))
    }

    fn parse_headers(info: &'_ str) -> Option<Vec<Header>> {
        let mut headers = vec![];

        for line in info.lines() {
            let name = line.split(":").take(1).collect::<String>();

            let value = line
                .split(":")
                .skip(1)
                .collect::<String>()
                .trim_start()
                .to_string();

            if name.len() == 0 || value.len() == 0 {
                None?
            }

            headers.push(Header { name, value });
        }

        Some(headers)
    }
}

struct MessageBuilder {
    version: Version,
    code: Code,
    headers: Vec<Header>,
    payload: Vec<u8>,
}

impl MessageBuilder {
    pub fn new() -> Self {
        Self {
            version: Version { major: 1, minor: 1 },
            code: Code::Success,
            headers: vec![],
            payload: vec![],
        }
    }

    pub fn version(self, version: Version) -> Self {
        Self { version, ..self }
    }

    pub fn code(self, code: Code) -> Self {
        Self { code, ..self }
    }

    pub fn header(self, header: Header) -> Self {  
        let Self {
            mut headers,
            ..
        } = self;
        
        headers.push(header);

        Self { headers, ..self }
    }

    pub fn body(mut self, body: Body<'_>) -> Self {
        let Self {
            mut payload,
            ..
        } = self;
        
        payload.extend(body.bytes());

        Self { payload, ..self }
    }

    pub fn build(self) -> Message {
        let MessageBuilder {
            version,
            code,
            headers,
            payload,
        } = self;

        let mut frames = vec![];

        let headers = Frame::Headers { headers };

        frames.push(headers);

        let data = Frame::Data { payload };

        frames.push(data);

        Message::Response {
            version,
            code,
            frames,
        }
    }
}

struct Header {
    pub name: String,
    pub value: String,
}

macro_rules! headers { 
    ($builder: ident, $($name: literal: $value: expr),*) => {
        $($builder = $builder.header(Header { name: format!("{}", { $name }), value: format!("{}", { $value }) });)*
    };
}

macro_rules! body { 
    ($builder: ident, $body: expr) => {
        $builder = $builder.body($body)
    };
}

enum Frame {
    Headers { headers: Vec<Header> },
    Data { payload: Vec<u8> },
}

pub struct Server {
    listener: net::TcpListener,
}

impl Server {
    pub fn bind(port: u16) -> Result<Self> {
        let local_host = String::from(LOCAL_HOST);
        let addr = local_host + ":" + &port.to_string();

        let listener = net::TcpListener::bind(addr).map_err(|e| Error::from(e))?;

        Ok(Self { listener })
    }

    pub fn respond(&mut self) {
        use io::{ Write, Read };
        
        let Some((mut stream, addr)) = self.listener.accept().ok() else {
            return;
        };

        stream.set_nonblocking(true).expect("failed to set stream to non-blocking");
        
        let mut buffer = vec![];

        let mut octet = [0; 8];

        loop {
            let read = stream.read(&mut octet);

            if let Ok(length) = read {
                if length == 0 {
                    break;
                }

                for i in 0..length {
                    buffer.push(octet[i]);
                }
            } else if let Err(e) = read {
                break;
            }
        }

        let request = Message::parse(&buffer);

        let body = "
            <html>
                <p>Hello, world!</p>
            </html>
        ";

        const version: &'static str = "0.1";

        let mut message = MessageBuilder::new();

        headers! { message, 
            "Server": format!("{}/{}", "Persona", version),
            "Content-type": "text/html", 
            "Content-Length": body.bytes().len()
        };

        body! { message, body };

        let response = message.build().into_bytes();

        stream.write(&response).expect("failed to write to stream");

        stream.shutdown(net::Shutdown::Write).expect("failed to shutdown stream");
    }
}
