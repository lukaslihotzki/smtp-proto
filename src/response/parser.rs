use std::slice::Iter;

use crate::{request::parser::Rfc5321Parser, *};

use super::*;

pub const MAX_REPONSE_LENGTH: usize = 4096;

#[derive(Default)]
pub struct ResponseReceiver {
    buf: Vec<u8>,
    code: [u8; 6],
    is_last: bool,
    pos: usize,
}

impl ResponseReceiver {
    pub fn from_code(code: [u8; 3]) -> Self {
        Self {
            code: [code[0], code[1], code[2], 0, 0, 0],
            pos: 3,
            is_last: false,
            buf: Vec::new(),
        }
    }

    pub fn parse(&mut self, bytes: &mut Iter<'_, u8>) -> Result<Response<String>, Error> {
        for &ch in bytes {
            match self.pos {
                0..=2 => {
                    if ch.is_ascii_digit() {
                        if self.buf.is_empty() {
                            self.code[self.pos] = ch - b'0';
                        }
                        self.pos += 1;
                    } else {
                        return Err(Error::SyntaxError {
                            syntax: "Invalid response code",
                        });
                    }
                }
                3 => match ch {
                    b' ' => {
                        self.is_last = true;
                        self.pos += 1;
                    }
                    b'-' => {
                        self.pos += 1;
                    }
                    b'\r' => {
                        continue;
                    }
                    b'\n' => {
                        self.is_last = true;
                    }
                    _ => {
                        return Err(Error::SyntaxError {
                            syntax: "Invalid response separator",
                        });
                    }
                },
                4 | 5 | 6 => match ch {
                    b'0'..=b'9' => {
                        if self.buf.is_empty() {
                            let code = &mut self.code[self.pos - 1];
                            *code = code.saturating_mul(10).saturating_add(ch - b'0');
                        }
                    }
                    b'.' if self.pos < 6 && self.code[self.pos - 1] > 0 => {
                        self.pos += 1;
                    }
                    _ => {
                        if !ch.is_ascii_whitespace() {
                            self.buf.push(ch);
                        }
                        self.pos = 7;
                    }
                },
                _ => match ch {
                    b'\r' | b'\n' => (),
                    _ => {
                        if self.buf.len() < MAX_REPONSE_LENGTH {
                            self.buf.push(ch);
                        } else {
                            return Err(Error::ResponseTooLong);
                        }
                    }
                },
            }

            if ch == b'\n' {
                if self.is_last {
                    return Ok(Response {
                        code: [self.code[0], self.code[1], self.code[2]],
                        esc: [self.code[3], self.code[4], self.code[5]],
                        message: std::mem::take(&mut self.buf).into_string(),
                    });
                } else {
                    self.buf.push(b'\n');
                    self.pos = 0;
                }
            }
        }

        Err(Error::NeedsMoreData { bytes_left: 0 })
    }

    pub fn reset(&mut self) {
        self.is_last = false;
        self.code.fill(0);
        self.pos = 0;
    }
}

impl EhloResponse<String> {
    pub fn parse(bytes: &mut Iter<'_, u8>) -> Result<Self, Error> {
        let mut parser = Rfc5321Parser::new(bytes);
        let mut response = EhloResponse::default();
        let mut code = [0u8; 3];
        let mut eol = false;
        let mut is_first_line = true;

        while !eol {
            for code in code.iter_mut() {
                match parser.read_char()? {
                    ch @ b'0'..=b'9' => {
                        *code = ch - b'0';
                    }
                    _ => {
                        return Err(Error::SyntaxError {
                            syntax: "unexpected token",
                        });
                    }
                }
            }

            if code[0] != 2 || code[1] != 5 || code[2] != 0 {
                return Err(Error::InvalidResponse { code });
            }

            match parser.read_char()? {
                b' ' => {
                    eol = true;
                }
                b'-' => (),
                b'\n' if code[0] < 6 => {
                    break;
                }
                _ => {
                    return Err(Error::SyntaxError {
                        syntax: "unexpected token",
                    });
                }
            }

            if !is_first_line {
                response.capabilities |= match parser.hashed_value_long()? {
                    _8BITMIME => EXT_8BIT_MIME,
                    ATRN => EXT_ATRN,
                    AUTH => {
                        while parser.stop_char != LF {
                            if let Some(mechanism) = parser.mechanism()? {
                                response.auth_mechanisms |= mechanism;
                            }
                        }

                        EXT_AUTH
                    }
                    BINARYMIME => EXT_BINARY_MIME,
                    BURL => EXT_BURL,
                    CHECKPOINT => EXT_CHECKPOINT,
                    CHUNKING => EXT_CHUNKING,
                    CONNEG => EXT_CONNEG,
                    CONPERM => EXT_CONPERM,
                    DELIVERBY => {
                        response.deliver_by = if parser.stop_char != LF {
                            let db = parser.size()?;
                            if db != usize::MAX {
                                db as u64
                            } else {
                                0
                            }
                        } else {
                            0
                        };
                        EXT_DELIVER_BY
                    }
                    DSN => EXT_DSN,
                    ENHANCEDSTATUSCO
                        if parser.stop_char.to_ascii_uppercase() == b'D'
                            && parser.read_char()?.to_ascii_uppercase() == b'E'
                            && parser.read_char()?.to_ascii_uppercase() == b'S' =>
                    {
                        EXT_ENHANCED_STATUS_CODES
                    }
                    ETRN => EXT_ETRN,
                    EXPN => EXT_EXPN,
                    FUTURERELEASE => {
                        let max_interval = if parser.stop_char != LF {
                            parser.size()?
                        } else {
                            0
                        };
                        let max_datetime = if parser.stop_char != LF {
                            parser.size()?
                        } else {
                            0
                        };

                        response.future_release_interval = if max_interval != usize::MAX {
                            max_interval as u64
                        } else {
                            0
                        };
                        response.future_release_datetime = if max_datetime != usize::MAX {
                            max_datetime as u64
                        } else {
                            0
                        };
                        EXT_FUTURE_RELEASE
                    }
                    HELP => EXT_HELP,
                    MT_PRIORITY => {
                        response.mt_priority = if parser.stop_char != LF {
                            match parser.hashed_value_long()? {
                                MIXER => MtPriority::Mixer,
                                STANAG4406 => MtPriority::Stanag4406,
                                NSEP => MtPriority::Nsep,
                                _ => MtPriority::Mixer,
                            }
                        } else {
                            MtPriority::Mixer
                        };
                        EXT_MT_PRIORITY
                    }
                    MTRK => EXT_MTRK,
                    NO_SOLICITING => {
                        response.no_soliciting = if parser.stop_char != LF {
                            let text = parser.text()?;
                            if !text.is_empty() {
                                text.into()
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        EXT_NO_SOLICITING
                    }
                    ONEX => EXT_ONEX,
                    PIPELINING => EXT_PIPELINING,
                    REQUIRETLS => EXT_REQUIRE_TLS,
                    RRVS => EXT_RRVS,
                    SIZE => {
                        response.size = if parser.stop_char != LF {
                            let size = parser.size()?;
                            if size != usize::MAX {
                                size
                            } else {
                                0
                            }
                        } else {
                            0
                        };
                        EXT_SIZE
                    }
                    SMTPUTF8 => EXT_SMTP_UTF8,
                    STARTTLS => EXT_START_TLS,
                    VERB => EXT_VERB,
                    _ => 0,
                };
                parser.seek_lf()?;
            } else {
                let mut buf = Vec::with_capacity(16);
                loop {
                    match parser.read_char()? {
                        b'\n' => break,
                        b'\r' => (),
                        b' ' => {
                            parser.seek_lf()?;
                            break;
                        }
                        ch if buf.len() < MAX_REPONSE_LENGTH => {
                            buf.push(ch);
                        }
                        _ => return Err(Error::ResponseTooLong),
                    }
                }
                is_first_line = false;
                response.hostname = buf.into_string();
            }
        }

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    use super::ResponseReceiver;

    #[test]
    fn parse_ehlo() {
        for item in [
            (
                concat!(
                    "250-dbc.mtview.ca.us says hello\n",
                    "250-8BITMIME\n",
                    "250-ATRN\n",
                    "250-AUTH GSSAPI DIGEST-MD5 PLAIN\n",
                    "250-BINARYMIME\n",
                    "250-BURL imap\n",
                    "250-CHECKPOINT\n",
                    "250-CHUNKING\n",
                    "250-CONNEG\n",
                    "250-CONPERM\n",
                    "250-DELIVERBY\n",
                    "250-DSN\n",
                    "250-ENHANCEDSTATUSCODES\n",
                    "250-ETRN\n",
                    "250-EXPN\n",
                    "250-FUTURERELEASE 1234 5678\n",
                    "250-HELP\n",
                    "250-MT-PRIORITY\n",
                    "250-MTRK\n",
                    "250-NO-SOLICITING net.example:ADV\n",
                    "250-PIPELINING\n",
                    "250-REQUIRETLS\n",
                    "250-RRVS\n",
                    "250-SIZE 1000000\n",
                    "250-SMTPUTF8 ignore\n",
                    "250 STARTTLS\n",
                ),
                Ok(EhloResponse {
                    hostname: "dbc.mtview.ca.us".to_string(),
                    capabilities: EXT_8BIT_MIME
                        | EXT_ATRN
                        | EXT_AUTH
                        | EXT_BINARY_MIME
                        | EXT_BURL
                        | EXT_CHECKPOINT
                        | EXT_CHUNKING
                        | EXT_CONNEG
                        | EXT_CONPERM
                        | EXT_DELIVER_BY
                        | EXT_DSN
                        | EXT_ENHANCED_STATUS_CODES
                        | EXT_ETRN
                        | EXT_EXPN
                        | EXT_FUTURE_RELEASE
                        | EXT_HELP
                        | EXT_MT_PRIORITY
                        | EXT_MTRK
                        | EXT_NO_SOLICITING
                        | EXT_PIPELINING
                        | EXT_REQUIRE_TLS
                        | EXT_RRVS
                        | EXT_SIZE
                        | EXT_SMTP_UTF8
                        | EXT_START_TLS,
                    auth_mechanisms: AUTH_GSSAPI | AUTH_DIGEST_MD5 | AUTH_PLAIN,
                    deliver_by: 0,
                    future_release_interval: 1234,
                    future_release_datetime: 5678,
                    mt_priority: MtPriority::Mixer,
                    no_soliciting: Some("net.example:ADV".to_string()),
                    size: 1000000,
                }),
            ),
            (
                concat!(
                    "250-\n",
                    "250-DELIVERBY 240\n",
                    "250-FUTURERELEASE 123\n",
                    "250-MT-PRIORITY MIXER\n",
                    "250-NO-SOLICITING\n",
                    "250-SIZE\n",
                    "250 SMTPUTF8\n",
                ),
                Ok(EhloResponse {
                    hostname: "".to_string(),
                    capabilities: EXT_DELIVER_BY
                        | EXT_FUTURE_RELEASE
                        | EXT_MT_PRIORITY
                        | EXT_NO_SOLICITING
                        | EXT_SIZE
                        | EXT_SMTP_UTF8,
                    auth_mechanisms: 0,
                    deliver_by: 240,
                    future_release_interval: 123,
                    future_release_datetime: 0,
                    mt_priority: MtPriority::Mixer,
                    no_soliciting: None,
                    size: 0,
                }),
            ),
            (
                concat!(
                    "250-dbc.mtview.ca.us says hello\n",
                    "250-FUTURERELEASE\n",
                    "250 MT-PRIORITY STANAG4406\n",
                ),
                Ok(EhloResponse {
                    hostname: "dbc.mtview.ca.us".to_string(),
                    capabilities: EXT_FUTURE_RELEASE | EXT_MT_PRIORITY,
                    auth_mechanisms: 0,
                    deliver_by: 0,
                    future_release_interval: 0,
                    future_release_datetime: 0,
                    mt_priority: MtPriority::Stanag4406,
                    no_soliciting: None,
                    size: 0,
                }),
            ),
            (
                concat!("523-Massive\n", "523-Error\n", "523 Message\n"),
                Err(Error::UnknownCommand),
            ),
        ] {
            let (response, parsed_response): (&str, Result<EhloResponse<String>, Error>) = item;

            for replacement in ["", "\r\n", " \n", " \r\n"] {
                let response = if !replacement.is_empty() && parsed_response.is_ok() {
                    response.replace('\n', replacement)
                } else {
                    response.to_string()
                };
                assert_eq!(
                    parsed_response,
                    EhloResponse::parse(&mut response.as_bytes().iter()),
                    "failed for {:?}",
                    response
                );
            }
        }
    }

    #[test]
    fn parse_response() {
        let mut all_responses = Vec::new();
        let mut all_parsed_responses = Vec::new();

        for (response, parsed_response, _) in [
            (
                "250 2.1.1 Originator <ned@ymir.claremont.edu> ok\n",
                Response {
                    code: [2, 5, 0],
                    esc: [2, 1, 1],
                    message: "Originator <ned@ymir.claremont.edu> ok".to_string(),
                },
                true,
            ),
            (
                concat!(
                    "551-5.7.1 Forwarding to remote hosts disabled\n",
                    "551 5.7.1 Select another host to act as your forwarder\n"
                ),
                Response {
                    code: [5, 5, 1],
                    esc: [5, 7, 1],
                    message: concat!(
                        "Forwarding to remote hosts disabled\n",
                        "Select another host to act as your forwarder"
                    )
                    .to_string(),
                },
                true,
            ),
            (
                concat!(
                    "550-mailbox unavailable\n",
                    "550 user has moved with no forwarding address\n"
                ),
                Response {
                    code: [5, 5, 0],
                    esc: [0, 0, 0],
                    message: "mailbox unavailable\nuser has moved with no forwarding address"
                        .to_string(),
                },
                false,
            ),
            (
                concat!(
                    "550-mailbox unavailable\n",
                    "550 user has moved with no forwarding address\n"
                ),
                Response {
                    code: [5, 5, 0],
                    esc: [0, 0, 0],
                    message: "mailbox unavailable\nuser has moved with no forwarding address"
                        .to_string(),
                },
                true,
            ),
            (
                concat!(
                    "432-6.8.9\n",
                    "432-6.8.9 Hello\n",
                    "432-6.8.9 \n",
                    "432-6.8.9 ,\n",
                    "432-\n",
                    "432-6\n",
                    "432-6.\n",
                    "432-6.8\n",
                    "432-6.8.9\n",
                    "432 6.8.9 World!\n"
                ),
                Response {
                    code: [4, 3, 2],
                    esc: [6, 8, 9],
                    message: "\nHello\n\n,\n\n\n\n\n\nWorld!".to_string(),
                },
                true,
            ),
            (
                concat!("250-Missing space\n", "250\n", "250 Ignore this"),
                Response {
                    code: [2, 5, 0],
                    esc: [0, 0, 0],
                    message: "Missing space\n".to_string(),
                },
                true,
            ),
        ] {
            assert_eq!(
                parsed_response,
                ResponseReceiver::default()
                    .parse(&mut response.as_bytes().iter())
                    .unwrap(),
                "failed for {:?}",
                response
            );
            all_responses.extend_from_slice(response.as_bytes());
            all_parsed_responses.push(parsed_response);
        }

        // Test receiver
        for chunk_size in [5, 10, 20, 30, 40, 50, 60] {
            let mut receiver = ResponseReceiver::default();
            let mut parsed_response = all_parsed_responses.clone().into_iter();
            for chunk in all_responses.chunks(chunk_size) {
                let mut bytes = chunk.iter();
                loop {
                    match receiver.parse(&mut bytes) {
                        Ok(response) => {
                            assert_eq!(
                                parsed_response.next(),
                                Some(response),
                                "chunk size {}",
                                chunk_size
                            );
                            receiver.reset();
                        }
                        Err(Error::NeedsMoreData { .. }) => {
                            break;
                        }
                        err => panic!("Unexpected error {:?} for chunk size {}", err, chunk_size),
                    }
                }
            }
        }
    }
}
