use document_loader::LoadType;
use dom::bindings::cell::DOMRefCell;
use dom::bindings::codegen::Bindings::ServoHTMLParserBinding;
use dom::bindings::global::GlobalRef;
use dom::bindings::inheritance::Castable;
use dom::bindings::js::{JS, Root};
use dom::bindings::refcounted::Trusted;
use dom::bindings::reflector::{Reflector, Reflectable, reflect_dom_object};
use dom::bindings::trace::JSTraceable;
use dom::document::Document;
use dom::node::Node;
use dom::servohtmlparser::{ParserContext, ParserRoot, Sink, ServoHTMLParser, Tokenizer};
use dom::servoxmlparser::ServoXMLParser;
use dom::window::Window;
use encoding::all::UTF_8;
use encoding::types::{DecoderTrap, Encoding};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer;
use html5ever::tree_builder;
use html5ever::tree_builder::{TreeBuilder, TreeBuilderOpts, NodeOrText, QuirksMode};
use hyper::header::ContentType;
use hyper::mime::{Mime, SubLevel, TopLevel};
use js::jsapi::JSTracer;
use msg::constellation_msg::{PipelineId, SubpageId};
use net_traits::{AsyncResponseListener, Metadata, NetworkError};
use network_listener::PreInvoke;
use parse::html::{ParseNode, ParseNodeData, process_op, ServoAttribute, ServoNodeOrText};
use parse::Parser;
use script_runtime::ScriptChan;
use script_thread::{MainThreadScriptMsg, MainThreadScriptChan, ScriptThread};
use std::cell::Cell;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::default::Default;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::ptr;
use string_cache::QualName;
use url::Url;

pub enum ParserThreadMsg {
    ScriptToParserMsg(TokenizerOperation),
    ParserToScriptMsg(ParserOperation),
}

// Messages to Parser Thread
pub enum TokenizerOperation {
    Feed(StrTendril),
    Run,
    SetPlaintextState,
    End,
}

// Messgaes from Parser Thread
pub enum ParserOperation {
    GetTemplateContents(usize, usize),
    CreateElement(QualName, Vec<ServoAttribute>, usize),
    CreateComment(String, usize),
    Insert(usize, Option<usize>, ServoNodeOrText),
    AppendDoctypeToDocument(String, String, String),
    AddAttrsIfMissing(usize, Vec<ServoAttribute>),
    RemoveFromParent(usize),
    MarkScriptAlreadyStarted(usize),
    CompleteScript(usize),
    ReparentChild(usize, usize),
    SetQuirksMode(QuirksMode),
}

pub trait ParserChan {
    fn send(&self, msg: TokenizerOperation) -> Result<(), ()>;
    fn clone(&self) -> Box<ParserChan + Send>;
}

pub trait ParserPort {
    fn recv(&self) -> Result<TokenizerOperation, ()>;
}

#[derive(JSTraceable)]
#[ignore_heap_size_of="xxx"]
pub struct ParserThreadChan(pub Sender<TokenizerOperation>);

impl ParserChan for ParserThreadChan {
    fn send(&self, msg: TokenizerOperation) -> Result<(), ()> {
        self.0.send(msg).map_err(|_| ())
    }

    fn clone(&self) -> Box<ParserChan + Send> {
        box ParserThreadChan((&self.0).clone())
    }
}

impl ParserThreadChan {
    pub fn new() -> (Receiver<TokenizerOperation>, Box<ParserThreadChan>) {
        let (chan_, port_) = channel();
        (port_, box ParserThreadChan(chan_))
    }
}

impl ParserPort for Receiver<ParserThreadMsg> {
    fn recv(&self) -> Result<TokenizerOperation, ()> {
        match self.recv() {
            Ok(ParserThreadMsg::ScriptToParserMsg(msg)) => Ok(msg),
            Ok(_) => panic!("unexpected parser thread message!"),
            _ => Err(()),
        }
    }
}

// #[dom_struct]
#[allow(unrooted_must_root)]
pub struct ParserThread {
    port_: Receiver<TokenizerOperation>,
    chan_: ParserThreadChan,
    pending_input: DOMRefCell<Vec<String>>,
    tokenizer: DOMRefCell<Tokenizer>,
    trusted_document: Trusted<Document>,
}

impl ParserThread {
    pub fn new(base_url: Option<Url>, trusted_document: Trusted<Document>, pipeline: Option<PipelineId>,
        sink: Sink, chan: ParserThreadChan, chan_thread: MainThreadScriptChan){
        let (chan_, port_) = channel();
        thread::Builder::new().name("ParserThread".to_string()).spawn(move || {
            // let pending_input: DOMRefCell::new(vec!());

            let tb = TreeBuilder::new(sink, TreeBuilderOpts {
                ignore_missing_rules: true,
                .. Default::default()
            });

            let tokenizer = tokenizer::Tokenizer::new(tb, Default::default());

        });
    }

    // Handle instructions from Sender
    fn handle_instructions(&self) {
        loop {
            match self.port_.recv().unwrap() {
                TokenizerOperation::Feed(chunk) => {
                    self.tokenizer.borrow_mut().feed(chunk.into())
                }
                TokenizerOperation::Run => {
                    self.tokenizer.borrow_mut().run()
                }
                TokenizerOperation::SetPlaintextState => {
                    self.tokenizer.borrow_mut().set_plaintext_state()
                }
                TokenizerOperation::End => {
                    self.tokenizer.borrow_mut().end()
                }
                /* let mut pending_instructions = self.pending_instructions.borrow_mut();
                if !pending_instructions.is_empty() {
                    let instruct = pending_instructions.remove(0);
                    self.tokenizer.borrow_mut().feed(instruct.into());
                } else {
                    self.tokenizer.borrow_mut().run();
                }

                if pending_instructions.is_empty() {
                    break;
                }
                */
            }
        }
    }

    #[inline]
    pub fn tokenizer(&self) -> &DOMRefCell<Tokenizer> {
        &self.tokenizer
    }
    pub fn pending_input(&self) -> &DOMRefCell<Vec<String>> {
        &self.pending_input
    }
    // pub fn tokenizer(&ServoHTMLParser) -> &DOMRefCell<Tokenizer> {
    //     &ServoHTMLParser.tokenizer
    // }
    // pub fn pending_input(&ServoHTMLParser) -> &DOMRefCell<Vec<String>> {
    //     &ServoHTMLParser.pending_input
    // }
}
