//! The harness built into a product: that product's server hears a request
//! and hands it over, saying who asks.
//!
//! Nothing listens here. The product has a server of its own, people of its
//! own and a way in of its own; what it builds in is a Workbench per person,
//! each over a data root of its own, drawn under a path of the product's
//! server. Who a person is was settled before the request was handed over,
//! by the product, and the harness takes its word.

use std::sync::Arc;

use hyper::body::Bytes;
use hyper::{Request, Response};

use super::{Assembled, DataRoot};
use crate::workbench_shell::{SaidBy, ShellBody, asked, route_shell};
use crate::{CameBy, Principal, WorkbenchShellError, WorkbenchShellState};

/// Where a Workbench is drawn on the product's server.
#[derive(Clone, Debug, Default)]
pub struct BuiltIn {
    /// The path it is drawn under, as `/people/ada/agents`; nothing for
    /// the root.
    pub under: String,
}

/// A Workbench that answers what it is handed.
pub struct Answering {
    pub state: Arc<WorkbenchShellState>,
    pub root: DataRoot,
}

impl Assembled {
    /// Build this product into another: no door is opened, no listener is
    /// bound. Time is kept and what the chats were left owing is taken up,
    /// as when a door is opened.
    ///
    /// # Errors
    ///
    /// The owner cannot be named, or time cannot be kept.
    pub async fn built_in(self, at: BuiltIn) -> Result<Answering, String> {
        self.name_the_owner().await?;
        let failed = |error: WorkbenchShellError| error.to_string();
        self.state.build_into(&at.under);
        self.state
            .keep_time(&self.root.schedules())
            .await
            .map_err(failed)?;
        self.state.look_at_the_machine_meanwhile();
        self.state.take_up_chats().await.map_err(failed)?;
        Ok(Answering {
            state: self.state,
            root: self.root,
        })
    }
}

impl Answering {
    /// Answer a request the product's server heard, for somebody the
    /// product let in. Call it for nobody else: whoever it is called for
    /// has this Workbench's terminals and keys.
    pub async fn answer<Body>(&self, request: Request<Body>) -> Response<ShellBody>
    where
        Body: hyper::body::Body<Data = Bytes> + Send + 'static,
        Body::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let mut request = asked(request);
        request.extensions_mut().insert(SaidBy(Principal {
            by: CameBy::Embedder,
        }));
        Box::pin(route_shell(&self.state, request)).await
    }
}
