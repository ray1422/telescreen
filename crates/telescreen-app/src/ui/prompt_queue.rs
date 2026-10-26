//! A prompt presenter that queues the requests for the window layer. The window
//! layer makes the prompt windows, because only the event loop can make windows.

use std::cell::RefCell;
use std::rc::Rc;

use telescreen_core::ids::SessionId;
use telescreen_core::platform::DisplayInfo;
use telescreen_view::{PromptPresenter, PromptScreen};

#[derive(Debug, Clone)]
pub enum PromptRequest {
    Show {
        session: SessionId,
        display: DisplayInfo,
        screen: PromptScreen,
    },
    Hide {
        session: SessionId,
    },
}

#[derive(Debug, Clone, Default)]
pub struct PromptQueue(Rc<RefCell<Vec<PromptRequest>>>);

impl PromptQueue {
    pub fn take(&self) -> Vec<PromptRequest> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

impl PromptPresenter for PromptQueue {
    /// Accepts the request. If the window cannot be made, the window layer calls
    /// `Controller::prompt_failed`.
    fn show(
        &mut self,
        session: SessionId,
        display: &DisplayInfo,
        screen: &PromptScreen,
    ) -> Result<(), String> {
        self.0.borrow_mut().push(PromptRequest::Show {
            session,
            display: display.clone(),
            screen: screen.clone(),
        });
        Ok(())
    }

    fn hide(&mut self, session: SessionId) {
        self.0.borrow_mut().push(PromptRequest::Hide { session });
    }
}
