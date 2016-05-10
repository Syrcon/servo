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
use dom::servoxmlparser::ServoXMLParser;
use dom::window::Window;
use encoding::all::UTF_8;
use encoding::types::{DecoderTrap, Encoding};
use html5ever::tokenizer;
use html5ever::tree_builder;
use html5ever::tree_builder::{TreeBuilder, TreeBuilderOpts};
use hyper::header::ContentType;
use hyper::mime::{Mime, SubLevel, TopLevel};
use js::jsapi::JSTracer;
use net_traits::{AsyncResponseListener, Metadata, NetworkError};
use network_listener::PreInvoke;
use parse::html::{ParseNode, ParseNodeData, process_op, ParserOperation};
use parse::Parser;
use script_runtime::ScriptChan;
use script_thread::{MainThreadScriptMsg, ScriptThread};
use std::cell::Cell;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::default::Default;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::ptr;
use url::Url;


pub enum ParserMsg {
    // do I need to specify `self` here?
    Feed(StrTendril),
    Run,
    SetPlaintextState,
    End,
}

pub struct ParserThreadChan(pub Sender<ParserMsg>);

impl ParserChan for ParserThreadChan {
    fn send(&self, msg: ParserMsg) -> Result<(), ()> {
        self.0.send(ParserMsg::Parser(msg)).map_err(|_| ())
    }

    fn clone(&self) -> Box<ScriptChan + Send> {
        box ParserThreadChan((&self.0).clone())
    }
}

impl ParserThreadChan {
    pub fn new() -> (Receiver<ParserMsg>, Box<ParserThreadChan>) {
        let (chan_, port_) = channel();
        (port_, box ParserThreadChan(chan_))
    }
}

impl ScriptPort for Receiver<ParserMsg> {
    fn recv(&self) -> Result<ParserMsg, ()> {
        match self.recv() {
            Ok(ParserMsg(msg)) => Ok(msg),
            Ok(_) => panic!("unexpected parser thread event message!"),
            _ => Err(()),
        }
    }
}

#[allow(unrooted_must_root)]
pub struct ParserThread {
    port_: Receiver<ParserMsg>,
    chan_: ParserThreadChan,
    #[ignore_heap_size_of = "Defined in html5ever"]
    tokenizer: DOMRefCell<Tokenizer>,
}

impl ParserThread {
    fn new(base_url: Option<Url>, document: &Document, pipeline: Option<PipelineId>){
        let (chan_, port_) = channel();
        thread::Builder::new().name("ParserThread".to_string()).spawn(move || {
            let mut sink = Sink::new(base_url, document);

            sink.nodes.lock().unwrap().insert(0, JS::from_ref(document.upcast()));
            sink.parse_data.borrow_mut().lock().unwrap()..insert(0, ParseNodeData { qual_name: None, parent: None });

            let tb = TreeBuilder::new(sink, TreeBuilderOpts {
                ignore_missing_rules: true,
                .. Default::default()
            });

            let tok = tokenizer::Tokenizer::new(tb, Default::default());

            let parser = ServoHTMLParser {
                reflector_: Reflector::new(),
                tokenizer: DOMRefCell::new(tok),
                pending_input: DOMRefCell::new(vec!()),
                document: JS::from_ref(document),
                suspended: Cell::new(false),
                last_chunk_received: Cell::new(false),
                pipeline: pipeline,
                finished: Cell::new(false),
                nodes: Arc::new(Mutex::new(sink.nodes)),
                parse_data: Arc::new(Mutex::new(sink.parse_data)),
            };

            reflect_dom_object(box parser, GlobalRef::Window(document.window()),
                               ServoHTMLParserBinding::Wrap)

        }
    }

    // Handle instructions from Sender
    // Replace self.tokenizer with equivalent command
    fn handle_instructions(&self){
        loop {
            match self.port_.recv().unwrap() {
                ParserMsg::Feed(&self) => {
                    self.tokenizer.borrow_mut().feed(chunk.into())
                }
                ParserMsg::Run(&self) => {
                    self.tokenizer.borrow_mut().run()
                }
                ParserMsg::SetPlaintextState(&self) => {
                    self.tokenizer.borrow_mut().set_plaintext_state()
                }
                ParserMsg::End(&self) => {
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
}
