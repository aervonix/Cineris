//! A scripted provider that builds each request as a real protocol does
//! first ([`ScriptedProvider::built_as`]).
//!
//! The scripted provider takes any request. A request the real API would
//! refuse, because the protocol cannot build it (a thinking budget that is
//! not below `max_tokens`, a tool schema it cannot write), then passes a
//! test and fails a user. [`Built`] runs the real [`Protocol::build`] on
//! each request before it plays the script, and fails the request with the
//! build's error as [`ProviderClient`](cineris_llm::ProviderClient) does,
//! with nothing taken from the script. Its stable prefix is the protocol's
//! too, so the agent loop's debug-build check runs on real bytes.

use std::sync::{Arc, Mutex, PoisonError};

use cineris_base::CancelToken;
use cineris_llm::{
    BuildError, ModelClient, ModelEntry, ModelEvent, ModelRequest, ModelStream, Protocol,
    ProviderInfo,
};

use crate::ScriptedProvider;

/// A [`ScriptedProvider`] behind a real protocol's request builder.
#[derive(Clone)]
pub struct Built {
    inner: ScriptedProvider,
    protocol: Arc<dyn Protocol>,
    provider: ProviderInfo,
    model: Option<Arc<ModelEntry>>,
    refused: Arc<Mutex<Vec<(ModelRequest, BuildError)>>>,
}

impl std::fmt::Debug for Built {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Built")
            .field("protocol", &self.protocol.name())
            .field("provider", &self.provider.id)
            .finish_non_exhaustive()
    }
}

impl ScriptedProvider {
    /// This script, behind `protocol`'s request builder for `provider`.
    #[must_use]
    pub fn built_as(&self, protocol: Arc<dyn Protocol>, provider: ProviderInfo) -> Built {
        Built {
            inner: self.clone(),
            protocol,
            provider,
            model: None,
            refused: Arc::default(),
        }
    }
}

impl Built {
    /// Builds every request for `model` in place of the one it names: a
    /// test's fixture model is no model of the real provider's.
    #[must_use]
    pub fn with_model(mut self, model: Arc<ModelEntry>) -> Self {
        self.model = Some(model);
        self
    }

    /// The request as the protocol is given it.
    fn as_built(&self, request: &ModelRequest) -> ModelRequest {
        let mut request = request.clone();
        if let Some(model) = &self.model {
            request.model = Arc::clone(model);
        }
        request
    }

    /// The bytes the next request of `request`'s epoch must begin with.
    ///
    /// # Panics
    ///
    /// When the protocol cannot build it.
    #[must_use]
    pub fn render(&self, request: &ModelRequest) -> Vec<u8> {
        self.protocol
            .stable_prefix(&self.as_built(request), &self.provider)
            .expect("the request builds")
    }

    /// The requests the protocol refused to build, with why.
    #[must_use]
    pub fn refused(&self) -> Vec<(ModelRequest, BuildError)> {
        self.refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The protocol's name.
    #[must_use]
    pub fn protocol(&self) -> &str {
        self.protocol.name()
    }

    /// The script underneath.
    #[must_use]
    pub fn scripted(&self) -> &ScriptedProvider {
        &self.inner
    }
}

impl ModelClient for Built {
    fn stream(&self, request: ModelRequest, cancel: CancelToken) -> ModelStream {
        match self
            .protocol
            .build(&self.as_built(&request), &self.provider)
        {
            Ok(_) => self.inner.stream(request, cancel),
            Err(error) => {
                self.refused
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((request, error.clone()));
                ModelStream::from_events(vec![ModelEvent::Error(error.into())])
            }
        }
    }

    fn stable_prefix(&self, request: &ModelRequest) -> Option<Vec<u8>> {
        self.protocol
            .stable_prefix(&self.as_built(request), &self.provider)
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cineris_base::CancelToken;
    use cineris_llm::{
        BuildError, HttpFailure, HttpRequest, ModelClient, ModelEvent, ModelRequest, Protocol,
        ProviderError, ProviderInfo, StreamDecoder,
    };

    use crate::{Response, ScriptedProvider, TestProtocol, block_on, fixtures};

    /// The test protocol, but one that cannot build a request with a
    /// maximum output of 1: as Anthropic's cannot build a summary whose
    /// thinking budget is not below its `max_tokens`.
    #[derive(Debug)]
    struct Picky(TestProtocol);

    impl Protocol for Picky {
        fn name(&self) -> &str {
            self.0.name()
        }

        fn build(
            &self,
            request: &ModelRequest,
            provider: &ProviderInfo,
        ) -> Result<HttpRequest, BuildError> {
            if request.max_output == Some(1) {
                return Err(BuildError::new("the budget is not below max_tokens"));
            }
            self.0.build(request, provider)
        }

        fn decoder(
            &self,
            request: &ModelRequest,
            provider: &ProviderInfo,
        ) -> Box<dyn StreamDecoder> {
            self.0.decoder(request, provider)
        }

        fn classify(&self, failure: &HttpFailure<'_>, request: &ModelRequest) -> ProviderError {
            self.0.classify(failure, request)
        }

        fn stable_prefix(
            &self,
            request: &ModelRequest,
            provider: &ProviderInfo,
        ) -> Result<Vec<u8>, BuildError> {
            self.0.stable_prefix(request, provider)
        }
    }

    #[test]
    fn a_request_the_protocol_cannot_build_fails_and_takes_nothing_from_the_script() {
        let script = ScriptedProvider::new([Response::new().text("served")]);
        let built = script.built_as(
            Arc::new(Picky(TestProtocol)),
            ProviderInfo::new(
                fixtures::PROVIDER,
                fixtures::PROTOCOL,
                "https://test.invalid",
            ),
        );
        let mut refused = fixtures::request(fixtures::model());
        refused.max_output = Some(1);
        let events = block_on(built.stream(refused, CancelToken::new()).collect());
        assert!(
            matches!(events.as_slice(), [ModelEvent::Error(error)] if error.message.contains("max_tokens")),
            "{events:?}"
        );
        assert_eq!(built.refused().len(), 1);
        assert!(script.requests().is_empty(), "the script was asked");
        // A request it can build is served from the script.
        let events = block_on(
            built
                .stream(fixtures::request(fixtures::model()), CancelToken::new())
                .collect(),
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ModelEvent::Error(_))),
            "{events:?}"
        );
        assert_eq!(script.requests().len(), 1);
    }
}
