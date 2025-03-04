// Copyright 2019-2021 Parity Technologies (UK) Ltd.
// This file is part of Parity Bridges Common.

// Parity Bridges Common is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity Bridges Common is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity Bridges Common.  If not, see <http://www.gnu.org/licenses/>.

//! Tests for finality synchronization loop.

#![cfg(test)]

use crate::{
	finality_loop::{
		prune_recent_finality_proofs, read_finality_proofs_from_stream, run_loop_iteration,
		run_until_connection_lost, select_better_recent_finality_proof, select_header_to_submit,
		FinalityLoopState, FinalityProofs, FinalitySyncParams, RestartableFinalityProofsStream,
		SourceClient, TargetClient,
	},
	sync_loop_metrics::SyncLoopMetrics,
	FinalityProof, FinalitySyncPipeline, SourceHeader,
};

use async_trait::async_trait;
use futures::{FutureExt, Stream, StreamExt};
use parking_lot::Mutex;
use relay_utils::{
	relay_loop::Client as RelayClient, FailedClient, HeaderId, MaybeConnectionError,
	TrackedTransactionStatus, TransactionTracker,
};
use std::{
	collections::HashMap,
	pin::Pin,
	sync::Arc,
	time::{Duration, Instant},
};

type IsMandatory = bool;
type TestNumber = u64;
type TestHash = u64;

#[derive(Clone, Debug)]
struct TestTransactionTracker(TrackedTransactionStatus<HeaderId<TestHash, TestNumber>>);

impl Default for TestTransactionTracker {
	fn default() -> TestTransactionTracker {
		TestTransactionTracker(TrackedTransactionStatus::Finalized(Default::default()))
	}
}

#[async_trait]
impl TransactionTracker for TestTransactionTracker {
	type HeaderId = HeaderId<TestHash, TestNumber>;

	async fn wait(self) -> TrackedTransactionStatus<HeaderId<TestHash, TestNumber>> {
		self.0
	}
}

#[derive(Debug, Clone)]
enum TestError {
	NonConnection,
}

impl MaybeConnectionError for TestError {
	fn is_connection_error(&self) -> bool {
		false
	}
}

#[derive(Debug, Clone)]
struct TestFinalitySyncPipeline;

impl FinalitySyncPipeline for TestFinalitySyncPipeline {
	const SOURCE_NAME: &'static str = "TestSource";
	const TARGET_NAME: &'static str = "TestTarget";

	type Hash = TestHash;
	type Number = TestNumber;
	type Header = TestSourceHeader;
	type FinalityProof = TestFinalityProof;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestSourceHeader(IsMandatory, TestNumber, TestHash);

impl SourceHeader<TestHash, TestNumber> for TestSourceHeader {
	fn hash(&self) -> TestHash {
		self.2
	}

	fn number(&self) -> TestNumber {
		self.1
	}

	fn is_mandatory(&self) -> bool {
		self.0
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestFinalityProof(TestNumber);

impl FinalityProof<TestNumber> for TestFinalityProof {
	fn target_header_number(&self) -> TestNumber {
		self.0
	}
}

#[derive(Debug, Clone, Default)]
struct ClientsData {
	source_best_block_number: TestNumber,
	source_headers: HashMap<TestNumber, (TestSourceHeader, Option<TestFinalityProof>)>,
	source_proofs: Vec<TestFinalityProof>,

	target_best_block_id: HeaderId<TestHash, TestNumber>,
	target_headers: Vec<(TestSourceHeader, TestFinalityProof)>,
	target_transaction_tracker: TestTransactionTracker,
}

#[derive(Clone)]
struct TestSourceClient {
	on_method_call: Arc<dyn Fn(&mut ClientsData) + Send + Sync>,
	data: Arc<Mutex<ClientsData>>,
}

#[async_trait]
impl RelayClient for TestSourceClient {
	type Error = TestError;

	async fn reconnect(&mut self) -> Result<(), TestError> {
		unreachable!()
	}
}

#[async_trait]
impl SourceClient<TestFinalitySyncPipeline> for TestSourceClient {
	type FinalityProofsStream = Pin<Box<dyn Stream<Item = TestFinalityProof> + 'static + Send>>;

	async fn best_finalized_block_number(&self) -> Result<TestNumber, TestError> {
		let mut data = self.data.lock();
		(self.on_method_call)(&mut data);
		Ok(data.source_best_block_number)
	}

	async fn header_and_finality_proof(
		&self,
		number: TestNumber,
	) -> Result<(TestSourceHeader, Option<TestFinalityProof>), TestError> {
		let mut data = self.data.lock();
		(self.on_method_call)(&mut data);
		data.source_headers.get(&number).cloned().ok_or(TestError::NonConnection)
	}

	async fn finality_proofs(&self) -> Result<Self::FinalityProofsStream, TestError> {
		let mut data = self.data.lock();
		(self.on_method_call)(&mut data);
		Ok(futures::stream::iter(data.source_proofs.clone()).boxed())
	}
}

#[derive(Clone)]
struct TestTargetClient {
	on_method_call: Arc<dyn Fn(&mut ClientsData) + Send + Sync>,
	data: Arc<Mutex<ClientsData>>,
}

#[async_trait]
impl RelayClient for TestTargetClient {
	type Error = TestError;

	async fn reconnect(&mut self) -> Result<(), TestError> {
		unreachable!()
	}
}

#[async_trait]
impl TargetClient<TestFinalitySyncPipeline> for TestTargetClient {
	type TransactionTracker = TestTransactionTracker;

	async fn best_finalized_source_block_id(
		&self,
	) -> Result<HeaderId<TestHash, TestNumber>, TestError> {
		let mut data = self.data.lock();
		(self.on_method_call)(&mut data);
		Ok(data.target_best_block_id)
	}

	async fn submit_finality_proof(
		&self,
		header: TestSourceHeader,
		proof: TestFinalityProof,
	) -> Result<TestTransactionTracker, TestError> {
		let mut data = self.data.lock();
		(self.on_method_call)(&mut data);
		data.target_best_block_id = HeaderId(header.number(), header.hash());
		data.target_headers.push((header, proof));
		(self.on_method_call)(&mut data);
		Ok(data.target_transaction_tracker.clone())
	}
}

fn prepare_test_clients(
	exit_sender: futures::channel::mpsc::UnboundedSender<()>,
	state_function: impl Fn(&mut ClientsData) -> bool + Send + Sync + 'static,
	source_headers: HashMap<TestNumber, (TestSourceHeader, Option<TestFinalityProof>)>,
) -> (TestSourceClient, TestTargetClient) {
	let internal_state_function: Arc<dyn Fn(&mut ClientsData) + Send + Sync> =
		Arc::new(move |data| {
			if state_function(data) {
				exit_sender.unbounded_send(()).unwrap();
			}
		});
	let clients_data = Arc::new(Mutex::new(ClientsData {
		source_best_block_number: 10,
		source_headers,
		source_proofs: vec![TestFinalityProof(12), TestFinalityProof(14)],

		target_best_block_id: HeaderId(5, 5),
		target_headers: vec![],
		target_transaction_tracker: TestTransactionTracker(TrackedTransactionStatus::Finalized(
			Default::default(),
		)),
	}));
	(
		TestSourceClient {
			on_method_call: internal_state_function.clone(),
			data: clients_data.clone(),
		},
		TestTargetClient { on_method_call: internal_state_function, data: clients_data },
	)
}

fn test_sync_params() -> FinalitySyncParams {
	FinalitySyncParams {
		tick: Duration::from_secs(0),
		recent_finality_proofs_limit: 1024,
		stall_timeout: Duration::from_secs(1),
		only_mandatory_headers: false,
	}
}

fn run_sync_loop(
	state_function: impl Fn(&mut ClientsData) -> bool + Send + Sync + 'static,
) -> (ClientsData, Result<(), FailedClient>) {
	let (exit_sender, exit_receiver) = futures::channel::mpsc::unbounded();
	let (source_client, target_client) = prepare_test_clients(
		exit_sender,
		state_function,
		vec![
			(5, (TestSourceHeader(false, 5, 5), None)),
			(6, (TestSourceHeader(false, 6, 6), None)),
			(7, (TestSourceHeader(false, 7, 7), Some(TestFinalityProof(7)))),
			(8, (TestSourceHeader(true, 8, 8), Some(TestFinalityProof(8)))),
			(9, (TestSourceHeader(false, 9, 9), Some(TestFinalityProof(9)))),
			(10, (TestSourceHeader(false, 10, 10), None)),
		]
		.into_iter()
		.collect(),
	);
	let sync_params = test_sync_params();

	let clients_data = source_client.data.clone();
	let result = async_std::task::block_on(run_until_connection_lost(
		source_client,
		target_client,
		sync_params,
		None,
		exit_receiver.into_future().map(|(_, _)| ()),
	));

	let clients_data = clients_data.lock().clone();
	(clients_data, result)
}

#[test]
fn finality_sync_loop_works() {
	let (client_data, result) = run_sync_loop(|data| {
		// header#7 has persistent finality proof, but it isn't mandatory => it isn't submitted,
		// because header#8 has persistent finality proof && it is mandatory => it is submitted
		// header#9 has persistent finality proof, but it isn't mandatory => it is submitted,
		// because   there are no more persistent finality proofs
		//
		// once this ^^^ is done, we generate more blocks && read proof for blocks 12 and 14 from
		// the stream
		if data.target_best_block_id.0 == 9 {
			data.source_best_block_number = 14;
			data.source_headers.insert(11, (TestSourceHeader(false, 11, 11), None));
			data.source_headers
				.insert(12, (TestSourceHeader(false, 12, 12), Some(TestFinalityProof(12))));
			data.source_headers.insert(13, (TestSourceHeader(false, 13, 13), None));
			data.source_headers
				.insert(14, (TestSourceHeader(false, 14, 14), Some(TestFinalityProof(14))));
		}
		// once this ^^^ is done, we generate more blocks && read persistent proof for block 16
		if data.target_best_block_id.0 == 14 {
			data.source_best_block_number = 17;
			data.source_headers.insert(15, (TestSourceHeader(false, 15, 15), None));
			data.source_headers
				.insert(16, (TestSourceHeader(false, 16, 16), Some(TestFinalityProof(16))));
			data.source_headers.insert(17, (TestSourceHeader(false, 17, 17), None));
		}

		data.target_best_block_id.0 == 16
	});

	assert_eq!(result, Ok(()));
	assert_eq!(
		client_data.target_headers,
		vec![
			// before adding 11..14: finality proof for mandatory header#8
			(TestSourceHeader(true, 8, 8), TestFinalityProof(8)),
			// before adding 11..14: persistent finality proof for non-mandatory header#9
			(TestSourceHeader(false, 9, 9), TestFinalityProof(9)),
			// after adding 11..14: ephemeral finality proof for non-mandatory header#14
			(TestSourceHeader(false, 14, 14), TestFinalityProof(14)),
			// after adding 15..17: persistent finality proof for non-mandatory header#16
			(TestSourceHeader(false, 16, 16), TestFinalityProof(16)),
		],
	);
}

fn run_only_mandatory_headers_mode_test(
	only_mandatory_headers: bool,
	has_mandatory_headers: bool,
) -> Option<(TestSourceHeader, TestFinalityProof)> {
	let (exit_sender, _) = futures::channel::mpsc::unbounded();
	let (source_client, target_client) = prepare_test_clients(
		exit_sender,
		|_| false,
		vec![
			(6, (TestSourceHeader(false, 6, 6), Some(TestFinalityProof(6)))),
			(7, (TestSourceHeader(false, 7, 7), Some(TestFinalityProof(7)))),
			(8, (TestSourceHeader(has_mandatory_headers, 8, 8), Some(TestFinalityProof(8)))),
			(9, (TestSourceHeader(false, 9, 9), Some(TestFinalityProof(9)))),
			(10, (TestSourceHeader(false, 10, 10), Some(TestFinalityProof(10)))),
		]
		.into_iter()
		.collect(),
	);
	async_std::task::block_on(select_header_to_submit(
		&source_client,
		&target_client,
		&mut RestartableFinalityProofsStream::from(futures::stream::empty().boxed()),
		&mut vec![],
		10,
		5,
		&FinalitySyncParams {
			tick: Duration::from_secs(0),
			recent_finality_proofs_limit: 0,
			stall_timeout: Duration::from_secs(0),
			only_mandatory_headers,
		},
	))
	.unwrap()
}

#[test]
fn select_header_to_submit_skips_non_mandatory_headers_when_only_mandatory_headers_are_required() {
	assert_eq!(run_only_mandatory_headers_mode_test(true, false), None);
	assert_eq!(
		run_only_mandatory_headers_mode_test(false, false),
		Some((TestSourceHeader(false, 10, 10), TestFinalityProof(10))),
	);
}

#[test]
fn select_header_to_submit_selects_mandatory_headers_when_only_mandatory_headers_are_required() {
	assert_eq!(
		run_only_mandatory_headers_mode_test(true, true),
		Some((TestSourceHeader(true, 8, 8), TestFinalityProof(8))),
	);
	assert_eq!(
		run_only_mandatory_headers_mode_test(false, true),
		Some((TestSourceHeader(true, 8, 8), TestFinalityProof(8))),
	);
}

#[test]
fn select_better_recent_finality_proof_works() {
	// if there are no unjustified headers, nothing is changed
	assert_eq!(
		select_better_recent_finality_proof::<TestFinalitySyncPipeline>(
			&[(5, TestFinalityProof(5))],
			&mut vec![],
			Some((TestSourceHeader(false, 2, 2), TestFinalityProof(2))),
		),
		Some((TestSourceHeader(false, 2, 2), TestFinalityProof(2))),
	);

	// if there are no recent finality proofs, nothing is changed
	assert_eq!(
		select_better_recent_finality_proof::<TestFinalitySyncPipeline>(
			&[],
			&mut vec![TestSourceHeader(false, 5, 5)],
			Some((TestSourceHeader(false, 2, 2), TestFinalityProof(2))),
		),
		Some((TestSourceHeader(false, 2, 2), TestFinalityProof(2))),
	);

	// if there's no intersection between recent finality proofs and unjustified headers, nothing is
	// changed
	let mut unjustified_headers =
		vec![TestSourceHeader(false, 9, 9), TestSourceHeader(false, 10, 10)];
	assert_eq!(
		select_better_recent_finality_proof::<TestFinalitySyncPipeline>(
			&[(1, TestFinalityProof(1)), (4, TestFinalityProof(4))],
			&mut unjustified_headers,
			Some((TestSourceHeader(false, 2, 2), TestFinalityProof(2))),
		),
		Some((TestSourceHeader(false, 2, 2), TestFinalityProof(2))),
	);

	// if there's intersection between recent finality proofs and unjustified headers, but there are
	// no proofs in this intersection, nothing is changed
	let mut unjustified_headers = vec![
		TestSourceHeader(false, 8, 8),
		TestSourceHeader(false, 9, 9),
		TestSourceHeader(false, 10, 10),
	];
	assert_eq!(
		select_better_recent_finality_proof::<TestFinalitySyncPipeline>(
			&[(7, TestFinalityProof(7)), (11, TestFinalityProof(11))],
			&mut unjustified_headers,
			Some((TestSourceHeader(false, 2, 2), TestFinalityProof(2))),
		),
		Some((TestSourceHeader(false, 2, 2), TestFinalityProof(2))),
	);
	assert_eq!(
		unjustified_headers,
		vec![
			TestSourceHeader(false, 8, 8),
			TestSourceHeader(false, 9, 9),
			TestSourceHeader(false, 10, 10)
		]
	);

	// if there's intersection between recent finality proofs and unjustified headers and there's
	// a proof in this intersection:
	// - this better (last from intersection) proof is selected;
	// - 'obsolete' unjustified headers are pruned.
	let mut unjustified_headers = vec![
		TestSourceHeader(false, 8, 8),
		TestSourceHeader(false, 9, 9),
		TestSourceHeader(false, 10, 10),
	];
	assert_eq!(
		select_better_recent_finality_proof::<TestFinalitySyncPipeline>(
			&[(7, TestFinalityProof(7)), (9, TestFinalityProof(9))],
			&mut unjustified_headers,
			Some((TestSourceHeader(false, 2, 2), TestFinalityProof(2))),
		),
		Some((TestSourceHeader(false, 9, 9), TestFinalityProof(9))),
	);
}

#[test]
fn read_finality_proofs_from_stream_works() {
	// when stream is currently empty, nothing is changed
	let mut recent_finality_proofs = vec![(1, TestFinalityProof(1))];
	let mut stream = futures::stream::pending().into();
	read_finality_proofs_from_stream::<TestFinalitySyncPipeline, _>(
		&mut stream,
		&mut recent_finality_proofs,
	);
	assert_eq!(recent_finality_proofs, vec![(1, TestFinalityProof(1))]);
	assert!(!stream.needs_restart);

	// when stream has entry with target, it is added to the recent proofs container
	let mut stream = futures::stream::iter(vec![TestFinalityProof(4)])
		.chain(futures::stream::pending())
		.into();
	read_finality_proofs_from_stream::<TestFinalitySyncPipeline, _>(
		&mut stream,
		&mut recent_finality_proofs,
	);
	assert_eq!(recent_finality_proofs, vec![(1, TestFinalityProof(1)), (4, TestFinalityProof(4))]);
	assert!(!stream.needs_restart);

	// when stream has ended, we'll need to restart it
	let mut stream = futures::stream::empty().into();
	read_finality_proofs_from_stream::<TestFinalitySyncPipeline, _>(
		&mut stream,
		&mut recent_finality_proofs,
	);
	assert_eq!(recent_finality_proofs, vec![(1, TestFinalityProof(1)), (4, TestFinalityProof(4))]);
	assert!(stream.needs_restart);
}

#[test]
fn prune_recent_finality_proofs_works() {
	let original_recent_finality_proofs: FinalityProofs<TestFinalitySyncPipeline> = vec![
		(10, TestFinalityProof(10)),
		(13, TestFinalityProof(13)),
		(15, TestFinalityProof(15)),
		(17, TestFinalityProof(17)),
		(19, TestFinalityProof(19)),
	]
	.into_iter()
	.collect();

	// when there's proof for justified header in the vec
	let mut recent_finality_proofs = original_recent_finality_proofs.clone();
	prune_recent_finality_proofs::<TestFinalitySyncPipeline>(10, &mut recent_finality_proofs, 1024);
	assert_eq!(&original_recent_finality_proofs[1..], recent_finality_proofs,);

	// when there are no proof for justified header in the vec
	let mut recent_finality_proofs = original_recent_finality_proofs.clone();
	prune_recent_finality_proofs::<TestFinalitySyncPipeline>(11, &mut recent_finality_proofs, 1024);
	assert_eq!(&original_recent_finality_proofs[1..], recent_finality_proofs,);

	// when there are too many entries after initial prune && they also need to be pruned
	let mut recent_finality_proofs = original_recent_finality_proofs.clone();
	prune_recent_finality_proofs::<TestFinalitySyncPipeline>(10, &mut recent_finality_proofs, 2);
	assert_eq!(&original_recent_finality_proofs[3..], recent_finality_proofs,);

	// when last entry is pruned
	let mut recent_finality_proofs = original_recent_finality_proofs.clone();
	prune_recent_finality_proofs::<TestFinalitySyncPipeline>(19, &mut recent_finality_proofs, 2);
	assert_eq!(&original_recent_finality_proofs[5..], recent_finality_proofs,);

	// when post-last entry is pruned
	let mut recent_finality_proofs = original_recent_finality_proofs.clone();
	prune_recent_finality_proofs::<TestFinalitySyncPipeline>(20, &mut recent_finality_proofs, 2);
	assert_eq!(&original_recent_finality_proofs[5..], recent_finality_proofs,);
}

#[test]
fn different_forks_at_source_and_at_target_are_detected() {
	let (exit_sender, _exit_receiver) = futures::channel::mpsc::unbounded();
	let (source_client, target_client) = prepare_test_clients(
		exit_sender,
		|_| false,
		vec![
			(5, (TestSourceHeader(false, 5, 42), None)),
			(6, (TestSourceHeader(false, 6, 6), None)),
			(7, (TestSourceHeader(false, 7, 7), None)),
			(8, (TestSourceHeader(false, 8, 8), None)),
			(9, (TestSourceHeader(false, 9, 9), None)),
			(10, (TestSourceHeader(false, 10, 10), None)),
		]
		.into_iter()
		.collect(),
	);

	let mut progress = (Instant::now(), None);
	let mut finality_proofs_stream = RestartableFinalityProofsStream {
		needs_restart: false,
		stream: Box::pin(futures::stream::iter(vec![]).boxed()),
	};
	let mut recent_finality_proofs = Vec::new();
	let metrics_sync = SyncLoopMetrics::new(None, "source", "target").unwrap();
	async_std::task::block_on(run_loop_iteration::<TestFinalitySyncPipeline, _, _>(
		&source_client,
		&target_client,
		FinalityLoopState {
			progress: &mut progress,
			finality_proofs_stream: &mut finality_proofs_stream,
			recent_finality_proofs: &mut recent_finality_proofs,
			submitted_header_number: None,
		},
		&test_sync_params(),
		&Some(metrics_sync.clone()),
	))
	.unwrap();

	assert!(!metrics_sync.is_using_same_fork());
}

#[test]
fn stalls_when_transaction_tracker_returns_error() {
	let (_, result) = run_sync_loop(|data| {
		data.target_transaction_tracker = TestTransactionTracker(TrackedTransactionStatus::Lost);
		data.target_best_block_id = HeaderId(5, 5);
		data.target_best_block_id.0 == 16
	});

	assert_eq!(result, Err(FailedClient::Both));
}

#[test]
fn stalls_when_transaction_tracker_returns_finalized_but_transaction_fails() {
	let (_, result) = run_sync_loop(|data| {
		data.target_best_block_id = HeaderId(5, 5);
		data.target_best_block_id.0 == 16
	});

	assert_eq!(result, Err(FailedClient::Both));
}
