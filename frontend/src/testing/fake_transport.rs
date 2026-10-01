use crate::bridge::{BridgeSocket, BridgeTransport};
use openwebide_core::{BridgeClientMessage, BridgeServerMessage};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
pub struct FakeTransport {
    sockets: RefCell<Vec<Rc<FakeSocket>>>,
}

struct FakeSocket {
    sent: RefCell<Vec<String>>,
    on_message: Rc<dyn Fn(String)>,
    on_close: Rc<dyn Fn()>,
}

impl BridgeSocket for FakeSocket {
    fn send(&self, text: String) -> Result<(), String> {
        self.sent.borrow_mut().push(text);
        Ok(())
    }
}

impl BridgeTransport for FakeTransport {
    fn open(
        &self,
        _url: &str,
        on_message: Rc<dyn Fn(String)>,
        on_close: Rc<dyn Fn()>,
    ) -> Result<Rc<dyn BridgeSocket>, String> {
        let socket = Rc::new(FakeSocket {
            sent: RefCell::new(Vec::new()),
            on_message,
            on_close,
        });
        self.sockets.borrow_mut().push(socket.clone());
        Ok(socket)
    }
}

impl FakeTransport {
    pub fn connections(&self) -> usize {
        self.sockets.borrow().len()
    }
    pub fn sent(&self) -> Vec<BridgeClientMessage> {
        self.sockets
            .borrow()
            .iter()
            .flat_map(|socket| {
                socket
                    .sent
                    .borrow()
                    .iter()
                    .map(|text| serde_json::from_str(text).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    pub fn reply(&self, message: BridgeServerMessage) {
        let socket = self.sockets.borrow().last().unwrap().clone();
        (socket.on_message)(serde_json::to_string(&message).unwrap());
    }
    pub fn disconnect(&self) {
        let socket = self.sockets.borrow().last().unwrap().clone();
        (socket.on_close)();
    }
}
