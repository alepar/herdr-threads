#!/usr/bin/env python3
"""Parent-only V091 sealed exact-matrix enumeration and optional execution.

Additive current Main1-36/test proof; historical V090 baseline formats stay intact.

Preparation never invokes Cargo. Requires Python 3.11+, a fresh absolute evidence
directory, and an inventory refreshed to the final committed clean C source.
Default: enumerate only. --run: focused nextest, then six mandatory Cargo cases.
The frozen f330391 snapshot and explicit source-evidenced selection seal retain
all requirements. PREPARATION_ONLY metadata rejects Cargo before enumeration.
Only the parent may seal a clean committed successor after causal fix/review.
"""
from __future__ import annotations

import argparse
from collections import Counter
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import time
import tomllib
import uuid

ALLOWED = {
    "herdr-threads": ("lib", "herdr_threads"),
    "herdr-threads::combined": ("test", "combined"),
    "herdr-threads::hook_entrypoint": ("test", "hook_entrypoint"),
    "herdr-threads::integration": ("test", "integration"),
    "herdr-threads::service": ("test", "service"),
}
TARGET_ARGS = [
    "--lib", "--test", "combined", "--test", "hook_entrypoint",
    "--test", "integration", "--test", "service",
]
BASELINE_SHA256 = "d568da428d23aaa0b517d0ab6a656e0f1684634fdf18f4281761b12a35546fad"
HISTORICAL657_NAME = "resume-v090-9a573c3f-final-inventory.json"
HISTORICAL657_SHA256 = "c433673ebf52ce5d64b4352f8b9f6d512dcad60edb3a66d5034a7b22f699f3a8"
# Historical b6 Main244/391/657 facts below stay immutable. This is an additive
# current-Main acceptance floor, not a source-native full matrix or feature gate.
CURRENT_MAIN_BASE = '565def869ae50e47e6da7388ea333229050eaf8d'
CURRENT_MAIN_HEAD = '20e7ca73e41a9449376169ab6875c0675da02edc'
CURRENT_MAIN_TREE = '55565e591f37fbdf73d04ac8711d18857661920c'
CURRENT_MAIN_PROOF_SHA256 = 'cbd0eeea9d9cafec5b9e7e99a0d6ec1fc2f1c5586e183e351a78a50c0a63e9c8'
CURRENT_MAIN_NEW_KEYS = (('combined', 'direct_send::json_output_uses_the_versioned_send_envelope_with_the_thread'),
 ('combined', 'lazy_attention::a_sibling_lifecycle_check_in_resets_only_its_own_in_turn_service'),
 ('combined', 'lazy_attention::in_turn_display_of_a_nudge_owes_no_idle_wake'),
 ('combined', 'lazy_attention::in_turn_display_of_an_invitation_owes_no_idle_wake'),
 ('hook_entrypoint', 'lazy_mail_reaches_the_next_codex_user_prompt_hook'),
 ('hook_entrypoint', 'pair_accept_delivers_mixed_opening_mail_once'),
 ('hook_entrypoint', 'pair_rejoin_with_two_bindings_pins_read_positions'),
 ('lib', 'cli::hook_inbox::tests::in_turn_presentation_is_sent_only_to_a_daemon_that_records_it'),
 ('lib', 'scheduler::tests::new_attention_retries_a_refusal_backed_off_seat_at_once_and_keeps_the_backoff'),
 ('lib',
  'store::control::tests::direct_threads::accepting_a_pair_invitation_keeps_the_opening_messages_unread'),
 ('lib', 'store::control::tests::direct_threads::rejoin_never_reshows_a_lazy_row_displayed_out_of_order'),
 ('lib', 'store::wake_tests::a_binding_served_one_reason_is_still_woken_for_another'),
 ('lib', 'store::wake_tests::a_recovery_hint_wake_to_a_served_binding_is_refused'),
 ('lib', 'store::wake_tests::an_in_turn_offer_after_reservation_cancels_that_bindings_wake'),
 ('lib', 'store::wake_tests::in_turn_service_is_decided_for_the_binding_the_composer_reaches'))
CURRENT_MAIN_REQUIRED_KEYS = (('combined', 'contracts::wake_candidate_exposes_pending_obligation_with_zero_projection_bits'),
 ('combined', 'direct_send::json_output_uses_the_versioned_send_envelope_with_the_thread'),
 ('combined', 'golden_surfaces::contract_id_output'),
 ('combined', 'golden_surfaces::help_texts'),
 ('combined', 'golden_surfaces::hook_stdout_per_event'),
 ('combined', 'golden_surfaces::setup_reports'),
 ('combined', 'golden_surfaces::setup_status_reports'),
 ('combined', 'lazy26_prerequisite::fresh_store_installs26_and_reopens'),
 ('combined', 'lazy26_prerequisite::historical25_store_upgrade_preserves_evidence_and_defaults_modes'),
 ('combined', 'lazy_attention::a_sibling_lifecycle_check_in_resets_only_its_own_in_turn_service'),
 ('combined', 'lazy_attention::in_turn_display_of_a_nudge_owes_no_idle_wake'),
 ('combined', 'lazy_attention::in_turn_display_of_an_invitation_owes_no_idle_wake'),
 ('combined', 'lazy_schema::lazy_schema_upgrade25_defaults_existing_messages_to_ordinary'),
 ('combined', 'setup_cli::codex_setup_installs_user_hooks_and_unsetup_restores_bytes'),
 ('combined', 'topology_handoff::schema_tests::registered28_fresh_and_genuine_historical_chains_reopen'),
 ('combined',
  'topology_handoff::schema_tests::registered28_preserves25_and26_history_and_rolls_back_late_ddl_failure'),
 ('hook_entrypoint', 'lazy_mail_reaches_the_next_codex_tool_hook'),
 ('hook_entrypoint', 'lazy_mail_reaches_the_next_codex_user_prompt_hook'),
 ('hook_entrypoint', 'pair_accept_delivers_mixed_opening_mail_once'),
 ('hook_entrypoint', 'pair_rejoin_with_two_bindings_pins_read_positions'),
 ('integration',
  'channel_activation::channel_activation_real_daemon_loads_fresh_21_and_22_and_owns_archival_lane'),
 ('integration', 'session_bindings_smoke::config8_upgrade_from_a_v0_6_0_database'),
 ('lib', 'cli::hook_inbox::tests::cli_inbox_prints_first_contact_once'),
 ('lib', 'cli::hook_inbox::tests::in_turn_presentation_is_sent_only_to_a_daemon_that_records_it'),
 ('lib', 'cli::hook_inbox::tests::inbox_sends_claim_and_advances_after_full_flush'),
 ('lib', 'cli::hook_inbox::tests::subagent_inbox_never_sends_ack_displayed'),
 ('lib', 'cli::hook_inbox::tests::truncated_page_advances_only_what_was_displayed'),
 ('lib', 'harness::adapter_seam::adapter_seam_settings_kinds_name_their_adapter_and_declare_today_s_groups'),
 ('lib', 'harness::codex_tests::declaration_reports_native_transport_and_receipt_unsupported'),
 ('lib', 'harness::codex_tests::every_declared_hook_is_installed_by_setup_and_encodes_context_only'),
 ('lib', 'harness::contract_tests::adapters_local_dispatch_keeps_legacy_contract_id_bytes'),
 ('lib', 'harness::contract_tests::contract_ids_are_pinned'),
 ('lib', 'harness::setup_tests::codex_session_plan_registers_event_per_group'),
 ('lib', 'harness::setup_tests::codex_setup_registers_event_per_group'),
 ('lib', 'protocol::capabilities::tests::bare_daemon_advertises_legacy_capabilities_without_v2_recorder'),
 ('lib', 'protocol::capabilities::tests::capability_constants_are_stable'),
 ('lib', 'protocol::capabilities::tests::every_advertised_capability_has_a_handler'),
 ('lib', 'scheduler::tests::new_attention_retries_a_refusal_backed_off_seat_at_once_and_keeps_the_backoff'),
 ('lib', 'scheduler::wake_text_tests::a_recovery_hint_always_names_inbox'),
 ('lib', 'scheduler::wake_text_tests::a_seat_holding_parked_ids_is_woken_with_the_marker'),
 ('lib', 'scheduler::wake_text_tests::native_wake_types_the_bound_harness_wake_text'),
 ('lib', 'scheduler::wake_text_tests::registry_wake_text_is_the_marker_unless_the_policy_injects'),
 ('lib', 'scheduler::wake_text_tests::the_inbox_marker_follows_the_composer_bindings_parking'),
 ('lib', 'scheduler::wake_text_tests::wake_texts_are_one_line_and_the_minimal_one_is_content_free'),
 ('lib',
  'store::connection::tests::absorption_main24_and25_to26_preserves_diagnostics_and_matches_fresh_catalog'),
 ('lib', 'store::connection::tests::direct_pair_migration_preserves_v28_and_enforces_identity'),
 ('lib', 'store::connection::tests::user_intent_schema22_fresh_and_v21_upgrade'),
 ('lib', 'store::connection::tests::v28_upgrade_migrates_bindings_to_session_keyed_v32'),
 ('lib', 'store::connection::tests::v34_to_v35_moves_attention_frontiers_to_bindings'),
 ('lib',
  'store::control::tests::direct_threads::accepting_a_pair_invitation_keeps_the_opening_messages_unread'),
 ('lib', 'store::control::tests::direct_threads::rejoin_never_reshows_a_lazy_row_displayed_out_of_order'),
 ('lib', 'store::wake_tests::a_binding_served_one_reason_is_still_woken_for_another'),
 ('lib', 'store::wake_tests::a_recovery_hint_wake_to_a_served_binding_is_refused'),
 ('lib', 'store::wake_tests::an_in_turn_offer_after_reservation_cancels_that_bindings_wake'),
 ('lib', 'store::wake_tests::in_turn_service_is_decided_for_the_binding_the_composer_reaches'))
CURRENT_MAIN_MIGRATIONS = {'migrations/0001_initial.sql': {'git_blob_id': 'ffeaade84f5ad44efd496a5619a6ceeb06645d86',
                                 'sha256': '987b8ba240af579c64f6e622f35854f2e6b59cf7d2d28b61e02a487422ce59d2'},
 'migrations/0002_service_substrate.sql': {'git_blob_id': '6831f3a20f80393acaaaa0fb82da10ffd7f1b3f1',
                                           'sha256': '9b91d697bc713397e276e3814545b1934872cc90bd066e22293d5110ac550f1e'},
 'migrations/0003_invitation_cancellations.sql': {'git_blob_id': '481d3ef3cc8e25d0bcd48afdb150f0bdee07a5ad',
                                                  'sha256': '2b58325a2dfb5ae2ca25a01e3897d7e0dd8d62bcd61d5abf3d69250a502f2fac'},
 'migrations/0004_voluntary_membership.sql': {'git_blob_id': 'c88ddbdb4d3edafe61a43835e71a7c36e0369e43',
                                              'sha256': 'ace84ea5090fb9ba579641eab59cf2664846ee011d778df1e52bc3144b82eaa3'},
 'migrations/0005_service_notifications.sql': {'git_blob_id': 'ee8f04c5b4a1c96c1c2168c9fd220f34e580a792',
                                               'sha256': '4483d608cbdc871145114825d54da25d9ffae6b337a1037413226c27a63642fa'},
 'migrations/0006_retirement_health.sql': {'git_blob_id': '8438928a18d1afa2a9371f4232966f85a71d506c',
                                           'sha256': '6137f9393ef15758558c7dec39c5899dfcad9726d5a2cd50a780482c64cd5495'},
 'migrations/0007_attention_digest.sql': {'git_blob_id': 'c4297bd74419ef30333907be40cd4b9ebdf32579',
                                          'sha256': 'de9d2ccae1d86fef0a74c0215276a338268e00efafda3c7b2ef677c7000978dc'},
 'migrations/0008_digest_pending_paths.sql': {'git_blob_id': '22c6850ce559ea78e04a44d75dc200758c37c65c',
                                              'sha256': 'f1d0385323435a429e72478458cb1dbb1f3998dd7db3017677ba53d673740101'},
 'migrations/0009_human_occupant.sql': {'git_blob_id': 'be735bca5e0c5dfb0ea596fd4809b76d04d99f85',
                                        'sha256': '43a85a3642b1df9eecb613c84243a3b4432f45a8ab7ebed8ea5cf5904bf36c79'},
 'migrations/0010_b5_trust_guards.sql': {'git_blob_id': 'ba4bba6d2fa40cf7f8d2fefb638651db0837c564',
                                         'sha256': '1ae7ac8a64e3735e8c416123dccc82d3ecb443dc5bb73d4ddeab942e29d104ee'},
 'migrations/0011_cooperative_only.sql': {'git_blob_id': 'cdc8899d3f04bad6a5dd448d09ea5b0ac5ae2c86',
                                          'sha256': '7bc928a3203e851c3ad7ee3727a6933f95322a137e50b43a4c6fced5d7950011'},
 'migrations/0012_harness_version_evidence.sql': {'git_blob_id': '2760de22cfd67c7f331e6131851cb917479f703d',
                                                  'sha256': '30653883773fc1cbf4264f546fbc5d3254213227663bb8d521e61cd8084674d4'},
 'migrations/0013_thread_summaries.sql': {'git_blob_id': '6a719f36df8e972adbe8a3fd62b1b98e0f451b91',
                                          'sha256': 'ae82fc0dcdb58e35da333f6e3768d2dca1d1fd97981e8eb07f6d47238049c889'},
 'migrations/0014_catch_up_release.sql': {'git_blob_id': 'b742830cb46770ae93cb7fe9b1be555942ca174f',
                                          'sha256': '560c1247f23c045bd7db6f3aef854f5fab0b9d431f26f1e2b3f50b9e5eaff4aa'},
 'migrations/0015_preparation_retention.sql': {'git_blob_id': 'b326ac0c40645ad3bfc9ee97156b7e3035ffa02a',
                                               'sha256': '733197e3e2ec367ca8ab978c9e5fa1f84b5fd7020273f073d2efa491e9d27c14'},
 'migrations/0016_human_receipt_waivers.sql': {'git_blob_id': 'a70c4cef4aa13b1566a24d755d3b6323e8add989',
                                               'sha256': 'e5c9337a7a02c6c06dc0b08cf711298a4b0fdb75183719c4b2b1b0a59fb27b60'},
 'migrations/0017_wake_batches.sql': {'git_blob_id': '3de5632eef923c875c5c331b19f81cb758581fb0',
                                      'sha256': 'f28763acbbc032906e9864c5e52b14fdb40083d2b847f3b871553718c73ba115'},
 'migrations/0018_warning_conditions.sql': {'git_blob_id': '0e0d1b3a19f28dcaa58eff59b2d9923f07125697',
                                            'sha256': '4f9d4ba662067632bb1cd285c16a4a15bef73af3746e57d58781f90bebcc8b4f'},
 'migrations/0019_thread_names.sql': {'git_blob_id': '59df4103564b47fe547bffb0c1995908b6b88296',
                                      'sha256': '3dd24c210d32f35f703b7712c67dfff88563c645b0f2a912f2c5412f3f56a41b'},
 'migrations/0020_recent_activity.sql': {'git_blob_id': 'c0f0568ff6f0d9f69344de749079c1750c5b5390',
                                         'sha256': 'f771782df955e347412474ca40c189855adda0891672bc6df88905d9c50952a4'},
 'migrations/0021_invitation_rejections.sql': {'git_blob_id': 'c76679e0189972cdf3abe975f0be8e1c988398c1',
                                               'sha256': 'a6213407b257ffd92136dac2da78e73a65d8e227ef5b873fe0cf6576258531fb'},
 'migrations/0022_user_message_intent.sql': {'git_blob_id': 'e0ac9bfb33f252ef68bc2fb0c179a2624714bd37',
                                             'sha256': 'cdc20ce86f3c98179cae7eeb507b3d7ccedc384332eab04d1b6cff7f2279d384'},
 'migrations/0023_channel_archival.sql': {'git_blob_id': 'd0bc1aaee3a240aeaa4b2fb6f979e2952249ee74',
                                          'sha256': 'f10b178cebe67c226da3be753ff14bf6ac4facdd0fac65b23880d4047574125b'},
 'migrations/0024_harness_contract_diagnostics.sql': {'git_blob_id': 'cd149cf9eafdf0977d1ec9e965f7ffcae79ccc13',
                                                      'sha256': 'c7d919671587677085f1b7cf2ded8c9bbc8b4bd80b5683f70d4bc9f2a6a963d1'},
 'migrations/0025_warning_notice_delivery.sql': {'git_blob_id': 'f0cdb580c151abcdf42ae15fb091c96b220cded8',
                                                 'sha256': 'be0ff728fb17015517364604d2d2fe55a5a826665fb1f34e23ecf2f854431417'},
 'migrations/0026_lazy_message_delivery.sql': {'git_blob_id': 'd95b416092f47fc73632c3b4788da09a045999b9',
                                               'sha256': 'fc0d7cbef24a613fc5b96bebb06f653ed2af10b84424128d969b601e11b9f088'},
 'migrations/0027_harness_adapters.sql': {'git_blob_id': 'b7995bedb994959b3af912c7b357183569bbfdc2',
                                          'sha256': 'b755fbed6f9c0b61e9e2f1bbe8dcd7c5cd967d06a200fcf6fde383dbc1f25a7a'},
 'migrations/0028_handoff_topology.sql': {'git_blob_id': '01852f36265dae509c1183a50a7d6639a736e76b',
                                          'sha256': 'cadc1fbfd7246deeec836b9236a0e5b69486e630306fe31d2a5c0851bfb19f22'},
 'migrations/0029_direct_threads.sql': {'git_blob_id': '42d0f534cc1f7ef52bd1fcbf4ae21101d9d0ee69',
                                        'sha256': '8eb79b580a05c907e31854efeb3b8fe91dd493dff07230a31278457c8e39b112'},
 'migrations/0030_automatic_handoff.sql': {'git_blob_id': '49ca7cf1e47ce6a50b4f0b07894d267a4327ec51',
                                           'sha256': '3b3a7f837df0f53f5f7506243f6ee3bdef1f350e94708d0666a2aa904f51f644'},
 'migrations/0031_seat_wake_on.sql': {'git_blob_id': '1daf0194717c4d90c88871c25425fbd22b8c6b6a',
                                      'sha256': '32f9c35e3ebc657c6e4227454cd276c3fceebe6043b13a60cba4fad29f3f55f2'},
 'migrations/0032_session_bindings.sql': {'git_blob_id': 'b3f71eb889bc98919c81803adef6dd93d451a170',
                                          'sha256': '8c64d36344aceefb7fbbf82487d24d5a93d33da9d20297ad4547bf48abcd1033'},
 'migrations/0033_archival_scan_scope.sql': {'git_blob_id': 'ce711f97fc57193630f55a3604586332ebbb0b64',
                                             'sha256': '8cab27994ee53b25a5e08fb75b5c8dd18a5d0ac5ea48e06029abc85974f1309a'},
 'migrations/0034_binding_delivery_state.sql': {'git_blob_id': '349491bd789c562ae5750c0f90f24f2486d5701a',
                                                'sha256': '64636bb15a05555a37f418c1041bcc84782aba881e0fa17ca4c7b78d3d7e2d3b'},
 'migrations/0035_binding_attention_frontiers.sql': {'git_blob_id': '55d7cf7894162f2648783507bb1426233011d453',
                                                     'sha256': '628f64be5f80b7813445f79f2b747dcdffd2d6a3b64d463ff5c9a9cbc6dea960'},
 'migrations/0036_in_turn_offer.sql': {'git_blob_id': '5ed1fdc2a09ab4baf318f4822b4783315222c6b5',
                                       'sha256': 'dd17b386f774d238232f2d4d945a85621eb42d6b781a185a2c389704e7662e11'}}
CURRENT_MAIN_PROOF_INPUT = "git:" + CURRENT_MAIN_HEAD + ":required-test-registration-proof"
CURRENT_MAIN_MIGRATIONS_INPUT = "git:" + CURRENT_MAIN_HEAD + ":released-migrations-1-36"
CURRENT_MAIN_MIGRATIONS_SHA256 = '610b2afeaacf08b8190a5415a045d5c65a0bfcffbedea8cbf18a044f097bd02a'

# The native-event contract seam is feature-owned, absent from released Main.
FEATURE_FLOOR_HEAD = '0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2'
FEATURE_FLOOR_TREE = '29f7d42d742fa09a56bf64f9de1648420e2565e3'
FEATURE_FLOOR_PROOF_SHA256 = 'c4452c2eb8d717644ed585db8727bad87dc2821f79c6c0b8588dc4d4ab8aa252'
FEATURE_FLOOR_KEYS = (('hook_entrypoint',
  'native_event_entrypoints_dispatch_declared_codecs_and_refuse_invalid_pairs_before_effects'),)
FEATURE_FLOOR_INPUT = "git:" + FEATURE_FLOOR_HEAD + ":native-event-contract-proof"


LOG_LIMIT = 32 * 1024 * 1024
LIST_LIMIT = 64 * 1024 * 1024
RECORD_LIMIT = 8 * 1024 * 1024


class TerminationRequested(BaseException):
    def __init__(self, signum: int):
        self.signum = signum
        super().__init__(f"termination signal {signum}")


class Cancellation:
    """Catchable termination enters owned cleanup; repeat signals never skip it."""
    def __init__(self):
        self.signum = None
        self.depth = 0

    def handle(self, signum, _frame):
        first = self.signum is None
        if first:
            self.signum = signum
        if first and not self.depth:
            raise TerminationRequested(signum)

    @contextmanager
    def defer(self, raise_after=True):
        self.depth += 1
        try:
            yield
        finally:
            self.depth -= 1
            if (
                raise_after and not self.depth and self.signum is not None
                and sys.exc_info()[0] is None
            ):
                raise TerminationRequested(self.signum)


def terminate_owned_group(child, grace_seconds=5.0) -> None:
    """Only a Popen created with start_new_session=True by this runner."""
    pgid = child.pid
    try:
        os.killpg(pgid, signal.SIGTERM)
    except ProcessLookupError:
        if child.poll() is None:
            child.wait(timeout=grace_seconds)
        return
    deadline = time.monotonic() + grace_seconds
    # Reap the leader, but keep checking its owned group: descendants can
    # outlive Cargo, and a leader exit alone does not prove group shutdown.
    while True:
        if child.poll() is None:
            try:
                child.wait(timeout=max(0.001, min(0.05, deadline - time.monotonic())))
            except subprocess.TimeoutExpired:
                pass
        try:
            os.killpg(pgid, 0)
        except ProcessLookupError:
            break
        if time.monotonic() >= deadline:
            try:
                os.killpg(pgid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            break
        time.sleep(0.01)
    if child.poll() is None:
        child.wait()


def mandatory_argv(row: dict) -> list[str]:
    if row["binary_id"] != "herdr-threads::combined":
        raise ValueError("mandatory target must be combined")
    return [
        "nice", "cargo", "test", "--locked", "--all-features",
        "--test", "combined", row["name"], "--", "--exact",
    ]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def save(path: Path, value: object) -> None:
    encoded = (json.dumps(value, indent=2) + "\n").encode()
    if len(encoded) > RECORD_LIMIT:
        raise ValueError(f"record exceeds {RECORD_LIMIT} bytes: {path.name}")
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(encoded)
    temporary.replace(path)


def git(source: Path, *arguments: str) -> bytes:
    return subprocess.check_output(["git", *arguments], cwd=source)


def text_git(source: Path, *arguments: str) -> str:
    return git(source, *arguments).decode().strip()


def pair(row: dict) -> tuple[str, str]:
    return row["binary_id"], row["name"]


def predicate(row: dict) -> str:
    kind, binary_name = ALLOWED[row["binary_id"]]
    name = row["name"]
    # The refreshed preparation admits only Rust identifier module paths.
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*(?:::[A-Za-z_][A-Za-z_0-9]*)*", name):
        raise ValueError(f"unsafe or unsupported test name: {name}")
    return (
        f"(package(=herdr-threads) & kind({kind}) & "
        f"binary(={binary_name}) & test(={name}))"
    )


def selection_fingerprint(rows: list[dict]) -> str:
    fields = (
        "target", "binary_id", "binary_name", "target_kind", "name",
        "source_path", "source_line", "source_file_sha256",
        "source_function_sha256", "source_tree", "selection",
        "registration_chain", "test_attributes",
    )
    proof = [
        {key: row[key] for key in fields}
        for row in sorted(rows, key=pair)
    ]
    return hashlib.sha256(json.dumps(
        proof, sort_keys=True, separators=(",", ":")
    ).encode()).hexdigest()


def validate_inventory(
    inventory: dict, baseline: dict, seal: dict
) -> tuple[list[dict], list[dict]]:
    """Validate explicit source proof; pure function, including prototypes."""
    if seal.get("schema") != "herdr-threads.v090.selection-seal.v1":
        raise ValueError("wrong V090 selection seal schema")
    rows = inventory["expected_tests"]
    keys = [pair(row) for row in rows]
    if not rows or len(keys) != len(set(keys)):
        raise ValueError("empty or duplicate selection")
    original = {(row["target"], row["name"]): row for row in baseline["expected_tests"]}
    if len(original) != 391:
        raise ValueError("frozen original391 requirement set changed")
    dispositions = seal["reviewed_key_dispositions"]
    if len(dispositions) != 2:
        raise ValueError("both reviewed semantic dispositions must be retained")
    historical = [(row["target"], row["historical"]) for row in dispositions]
    if len(set(historical)) != len(historical):
        raise ValueError("duplicate semantic rename mapping")

    def normalize(key: tuple[str, str]) -> tuple[str, str]:
        for row in dispositions:
            if key == (row["target"], row["historical"]):
                return row["target"], row["canonical"]
        return key

    traces = seal["original391_requirement_map"]
    historical_traces = [(row["historical_target"], row["historical_name"]) for row in traces]
    if len(traces) != 391 or len(set(historical_traces)) != 391 or set(historical_traces) != set(original):
        raise ValueError("original391 traceability is incomplete or fabricated")
    canonical_by_target = {(row["target"], row["name"]) for row in rows}
    for trace in traces:
        old = trace["historical_target"], trace["historical_name"]
        actual = trace["canonical_target"], trace["canonical_name"]
        if actual != normalize(old) or actual not in canonical_by_target:
            raise ValueError(f"baseline requirement lost or incorrectly renamed: {old}")
        prior = original[old]
        if (
            trace["original_source_path"] != prior["source_path"]
            or trace["original_source_file_sha256"] != prior["source_file_sha256"]
            or trace["original_source_function_sha256"] != prior["source_function_sha256"]
        ):
            raise ValueError(f"historical baseline source fingerprints changed: {old}")
    for row in dispositions:
        old = row["target"], row["historical"]
        actual = row["target"], row["canonical"]
        if row["historical_was_in_original391"] != (old in original):
            raise ValueError("fabricated historical391 membership")
        if actual not in canonical_by_target:
            raise ValueError("canonical disposition requirement missing")
    allowed = seal["explicit_source_evidenced_allowlist"]
    allowed_keys = [pair(row) for row in allowed]
    if len(allowed_keys) != len(set(allowed_keys)) or set(keys) != set(allowed_keys):
        raise ValueError("selection differs from explicit frozen allowlist")
    if len(keys) < 391:
        raise ValueError("expanded V090 enumeration cannot shrink original391")
    if len(seal["published_Main244_keys"]) != 244:
        raise ValueError("Main concrete244 provenance is incomplete")
    for origin in seal["published_Main244_keys"] + seal["actual_composed_author_keys"]:
        if normalize((origin["target"], origin["name"])) not in canonical_by_target:
            raise ValueError("source-evidenced Main/author requirement missing")
    fingerprint = selection_fingerprint(rows)
    if fingerprint != seal["selection_fingerprint"] or fingerprint != inventory["selection_fingerprint"]:
        raise ValueError("source/registration/selection fingerprint mismatch")
    fields = (
        "target", "binary_id", "binary_name", "target_kind", "name",
        "source_path", "source_line", "source_file_sha256",
        "source_function_sha256", "source_tree", "selection",
        "registration_chain", "test_attributes",
    )
    proof = {pair(row): row for row in allowed}
    for row in rows:
        if any(row.get(key) != proof[pair(row)].get(key) for key in fields):
            raise ValueError(f"source proof changed: {pair(row)}")
        if row["binary_id"] not in ALLOWED:
            raise ValueError("unexpected target")
        kind, name = ALLOWED[row["binary_id"]]
        if (row["target_kind"], row["binary_name"]) != (kind, name):
            raise ValueError("wrong target kind/name")
        if not row.get("source_registered") or row["source_tree"] != inventory["source"]["tree"]:
            raise ValueError("unregistered or stale source row")
        if any(re.search(r"#\[\s*ignore", attr) for attr in row["test_attributes"]):
            raise ValueError("ignored source declaration cannot be selected")
        predicate(row)
    wanted_mandatory = {
        normalize((row["target"], row["name"])) for row in original.values()
        if row["selection"] == "mandatory_cargo_separate"
    }
    mandatory = [row for row in rows if row["selection"] == "mandatory_cargo_separate"]
    focused = [row for row in rows if row["selection"] == "focused_nextest"]
    if len(mandatory) != 6 or {(row["target"], row["name"]) for row in mandatory} != wanted_mandatory:
        raise ValueError("original mandatory six changed")
    if len(focused) + 6 != len(rows) or not focused:
        raise ValueError("unknown selection class or zero focused set")
    counts = inventory["counts"]
    if (counts["unique_expected_tests"], counts["mandatory_separate"], counts["focused_nextest"]) != (len(rows), 6, len(focused)):
        raise ValueError("static counts disagree with exact keys")
    if {row["binary_id"] for row in rows} != set(ALLOWED):
        raise ValueError("exact five targets required")
    return sorted(focused, key=pair), sorted(mandatory, key=pair)


def validate_current_main(inventory: dict, seal: dict) -> None:
    """Pin released Main source proof and concrete affected rows, never wildcard extras."""
    metadata = inventory.get("current_Main_v091")
    if not isinstance(metadata, dict) or metadata != seal.get("current_Main_v091"):
        raise ValueError("current Main proof differs or is absent")
    expected_identity = {
        "base": CURRENT_MAIN_BASE, "head": CURRENT_MAIN_HEAD,
        "tree": CURRENT_MAIN_TREE, "source_proof_sha256": CURRENT_MAIN_PROOF_SHA256,
    }
    if any(metadata.get(key) != value for key, value in expected_identity.items()):
        raise ValueError("wrong current Main proof identity")
    proof = metadata.get("source_proof", [])
    keys = [(row["target"], row["name"]) for row in proof]
    if len(keys) != len(set(keys)) or set(keys) != set(CURRENT_MAIN_REQUIRED_KEYS):
        raise ValueError("current Main affected exact floor changed")
    digest = hashlib.sha256(json.dumps(
        proof, sort_keys=True, separators=(",", ":")
    ).encode()).hexdigest()
    if digest != CURRENT_MAIN_PROOF_SHA256:
        raise ValueError("current Main immutable source/registration proof changed")
    if metadata.get("new_keys") != [
        {"target": target, "name": name} for target, name in CURRENT_MAIN_NEW_KEYS
    ]:
        raise ValueError("current Main fifteen new exact requirements changed")
    current = {(row["target"], row["name"]): row for row in inventory["expected_tests"]}
    for key in CURRENT_MAIN_REQUIRED_KEYS:
        if key not in current or current[key]["selection"] != "focused_nextest":
            raise ValueError(f"current Main focused source requirement missing: {key}")
    released = {
        "head": CURRENT_MAIN_HEAD, "tree": CURRENT_MAIN_TREE,
        "floor": 36, "source_files": CURRENT_MAIN_MIGRATIONS,
    }
    if inventory.get("released_Main_migrations") != released or seal.get("released_Main_migrations") != released:
        raise ValueError("released Main1-36 path/blob/hash provenance changed")
    for path, expected in CURRENT_MAIN_MIGRATIONS.items():
        if inventory["source_files"].get(path) != expected:
            raise ValueError(f"released Main migration missing or changed: {path}")
    for inputs in (inventory["history_input_sha256"], seal["input_sha256"]):
        if (
            inputs.get(CURRENT_MAIN_PROOF_INPUT) != CURRENT_MAIN_PROOF_SHA256
            or inputs.get(CURRENT_MAIN_MIGRATIONS_INPUT) != CURRENT_MAIN_MIGRATIONS_SHA256
        ):
            raise ValueError("current Main proof input hash changed or absent")


def validate_feature_floor(inventory: dict, seal: dict) -> None:
    """Retain the feature-owned native-event contract seam as an exact requirement."""
    metadata = inventory.get("feature_native_event_contract")
    if not isinstance(metadata, dict) or metadata != seal.get("feature_native_event_contract"):
        raise ValueError("feature native-event proof differs or is absent")
    if (
        metadata.get("head") != FEATURE_FLOOR_HEAD
        or metadata.get("tree") != FEATURE_FLOOR_TREE
        or metadata.get("source_proof_sha256") != FEATURE_FLOOR_PROOF_SHA256
    ):
        raise ValueError("wrong feature native-event source identity")
    proof = metadata.get("source_proof", [])
    keys = [(row["target"], row["name"]) for row in proof]
    if len(keys) != len(set(keys)) or set(keys) != set(FEATURE_FLOOR_KEYS):
        raise ValueError("feature native-event exact floor changed")
    if hashlib.sha256(json.dumps(
        proof, sort_keys=True, separators=(",", ":")
    ).encode()).hexdigest() != FEATURE_FLOOR_PROOF_SHA256:
        raise ValueError("feature native-event source/registration proof changed")
    current = {(row["target"], row["name"]): row for row in inventory["expected_tests"]}
    for key in FEATURE_FLOOR_KEYS:
        if key not in current or current[key]["selection"] != "focused_nextest":
            raise ValueError(f"feature native-event focused requirement missing: {key}")
    for inputs in (inventory["history_input_sha256"], seal["input_sha256"]):
        if inputs.get(FEATURE_FLOOR_INPUT) != FEATURE_FLOOR_PROOF_SHA256:
            raise ValueError("feature native-event proof input missing or changed")


def validate_historical657(
    inventory: dict, seal: dict, historical: dict, historical_path: Path
) -> None:
    """Retain every exact expanded657 requirement and its historical source proof."""
    prior_rows = historical["expected_tests"]
    prior = {(row["target"], row["name"]): row for row in prior_rows}
    if len(prior_rows) != 657 or len(prior) != 657:
        raise ValueError("frozen expanded657 requirement set changed")
    metadata = {
        "path": str(historical_path), "sha256": HISTORICAL657_SHA256,
        "original_keys_preserved": 657,
    }
    if inventory.get("historical657") != metadata or seal.get("historical657") != metadata:
        raise ValueError("missing or incorrect expanded657 inventory provenance")
    for inputs in (inventory["history_input_sha256"], seal["input_sha256"]):
        if inputs.get(str(historical_path)) != HISTORICAL657_SHA256:
            raise ValueError("expanded657 input hash missing or changed")
    traces = seal.get("historical657_requirement_map", [])
    trace_keys = [(row["historical_target"], row["historical_name"]) for row in traces]
    if len(traces) != 657 or len(set(trace_keys)) != 657 or set(trace_keys) != set(prior):
        raise ValueError("expanded657 traceability is incomplete or fabricated")
    current = {(row["target"], row["name"]): row for row in inventory["expected_tests"]}
    renames = {
        (row["target"], row["historical"]): (row["target"], row["canonical"])
        for row in seal["reviewed_key_dispositions"]
    }
    fields = (
        "source_path", "source_file_sha256", "source_function_sha256",
        "source_tree", "registration_chain", "selection",
    )
    for trace in traces:
        old = trace["historical_target"], trace["historical_name"]
        actual = trace["canonical_target"], trace["canonical_name"]
        if actual != renames.get(old, old) or actual not in current:
            raise ValueError(f"expanded657 requirement lost or incorrectly renamed: {old}")
        if trace["historical_source"] != {key: prior[old][key] for key in fields}:
            raise ValueError(f"expanded657 historical source proof changed: {old}")
        if current[actual]["selection"] != prior[old]["selection"]:
            raise ValueError(f"expanded657 selection class changed: {old}")


def validate_execution_seal(inventory: dict, seal: dict) -> None:
    """Parent final metadata only; rejects prototype before any Cargo."""
    final = "sealed_parent_reviewed_final_source"
    if (
        inventory.get("phase") != final or seal.get("phase") != final
        or inventory.get("preparation_only") is not False
        or inventory["source"].get("identity_kind") != "committed_frozen"
    ):
        raise ValueError("PREPARATION_ONLY staged metadata cannot enumerate or execute; refresh/seal only after parent causal fix/review and clean commit")
    if inventory.get("pending_required") or seal.get("pending_required"):
        raise ValueError("pending required source rows prevent enumeration/execution")
    if (
        not inventory["source"].get("head")
        or inventory["source"]["head"] != seal.get("source_head")
        or inventory["source"]["tree"] != seal.get("source_tree")
    ):
        raise ValueError("final committed source seal identity mismatch")
    negative = (
        "herdr-threads",
        "cli::topology_handoff::tests::live::automatic_live::automatic_child_begin_expired_permit_has_no_effects_and_fresh_permit_succeeds",
    )
    if negative not in {pair(row) for row in inventory["expected_tests"]}:
        raise ValueError("required actual imported71bb negative control is absent")
    if len(inventory["expected_tests"]) < 391:
        raise ValueError("standalone selected enumeration must preserve original391")


def inspect_selection(document: dict, expected: list[dict]) -> dict:
    expected_keys = {pair(row) for row in expected}
    suites = document.get("rust-suites")
    if not isinstance(suites, dict):
        raise ValueError("nextest JSON has no rust-suites")
    actual = []
    errors = []
    binaries = []
    for suite_key, suite in suites.items():
        binary = suite.get("binary-id")
        if binary != suite_key:
            errors.append(f"suite key/ID mismatch: {suite_key}")
        if binary not in ALLOWED:
            errors.append(f"unexpected suite: {binary}")
        elif (suite.get("kind"), suite.get("binary-name")) != ALLOWED[binary]:
            errors.append(f"wrong binary kind/name: {binary}")
        if suite.get("package-name") != "herdr-threads" or suite.get("status") != "listed":
            errors.append(f"wrong package or unlisted suite: {binary}")
        cases = suite.get("testcases")
        if not isinstance(cases, dict):
            errors.append(f"missing testcases: {binary}")
            continue
        for name, case in cases.items():
            match = case.get("filter-match", {}).get("status")
            if match not in {"matches", "mismatch"}:
                errors.append(f"unknown filter status: {binary}/{name}: {match}")
            if match != "matches":
                continue
            actual.append((binary, name))
            if case.get("kind") != "test" or case.get("ignored") is not False:
                errors.append(f"ignored or wrong-kind selected case: {binary}/{name}")
        binaries.append({
            "binary_id": binary, "binary_name": suite.get("binary-name"),
            "kind": suite.get("kind"), "path": suite.get("binary-path"),
            "build_platform": suite.get("build-platform"), "cwd": suite.get("cwd"),
        })
    if set(suites) != set(ALLOWED):
        errors.append("listing did not contain exactly the five selected suites")
    missing = sorted(expected_keys - set(actual))
    extra = sorted(set(actual) - expected_keys)
    duplicate = sorted(key for key, count in Counter(actual).items() if count > 1)
    return {
        "status": "matched" if actual and not (errors or missing or extra or duplicate) else "rejected",
        "expected_count": len(expected_keys), "selected_count": len(actual),
        "selected_by_target": dict(Counter(binary for binary, _ in actual)),
        "expected_exact_pairs": sorted(expected_keys), "actual_selected_pairs": sorted(actual),
        "missing": missing, "extra": extra, "duplicates": duplicate, "errors": errors,
        "listed_binaries": binaries, "reported_total_test_count": document.get("test-count"),
        "count_policy": "Exact matching binary/name pairs, never aggregate test-count.",
    }


def parse_nextest_results(path: Path, expected: list[dict]) -> dict:
    # Pinned to the observed default nextest status format; unknown output fails
    # evidence validation instead of inventing executed/pass counts.
    pattern = re.compile(
        r"^\s*(PASS|FAIL|TIMEOUT|LEAK|EXECFAIL|SKIP)\s+\[[^\]]+\]\s+"
        r"\(\s*\d+\s*/\s*\d+\s*\)\s+(\S+)\s+(\S+)\s*$"
    )
    results = []
    for line in path.read_text(errors="replace").splitlines():
        match = pattern.match(line)
        if match:
            status, binary, name = match.groups()
            results.append({"binary_id": binary, "name": name, "status": status})
    keys = [pair(row) for row in results]
    expected_keys = {pair(row) for row in expected}
    return {
        "executed_cases": results, "executed_count": len(results),
        "passed_count": sum(row["status"] == "PASS" for row in results),
        "statuses": dict(Counter(row["status"] for row in results)),
        "missing": sorted(expected_keys - set(keys)),
        "extra": sorted(set(keys) - expected_keys),
        "duplicates": sorted(key for key, count in Counter(keys).items() if count > 1),
        "evidence_complete": bool(results) and set(keys) == expected_keys and len(keys) == len(set(keys)),
        "parser_contract": "Observed nextest PASS/FAIL status lines; never infer counts from exit alone.",
    }


def parse_recovery_results(paths: Path | list[Path], expected: list[dict]) -> dict:
    """Account for every libtest result name, including unrelated namespaces."""
    if isinstance(paths, Path):
        paths = [paths]
    pattern = re.compile(r"^test (\S+) \.\.\. (.+)$")
    results, summaries, errors = [], [], []
    for path in paths:
        for line in path.read_text(errors="replace").splitlines():
            stripped = line.strip()
            match = pattern.match(stripped)
            if match:
                name, status = match.groups()
                if status.startswith("ignored"):
                    status = "ignored"
                elif status not in {"ok", "FAILED"}:
                    errors.append(f"unknown test status in {path.name}: {name}: {status}")
                results.append({
                    "binary_id": "herdr-threads::combined", "name": name,
                    "status": status, "log": str(path),
                })
            if stripped.startswith("test result:"):
                counts = dict((kind, int(number)) for number, kind in re.findall(
                    r"\b(\d+) (passed|failed|ignored|measured)\b", stripped
                ))
                summaries.append({"log": str(path), "line": stripped, "counts": counts})
                if set(counts) != {"passed", "failed", "ignored", "measured"}:
                    errors.append(f"unrecognized summary in {path.name}")
        if sum(summary["log"] == str(path) for summary in summaries) != 1:
            errors.append(f"expected one test summary in {path.name}")
    keys = [pair(row) for row in results]
    expected_keys = {pair(row) for row in expected}
    missing = sorted(expected_keys - set(keys))
    extra = sorted(set(keys) - expected_keys)
    duplicates = sorted(key for key, count in Counter(keys).items() if count > 1)
    ignored = [row["name"] for row in results if row["status"] == "ignored"]
    totals = Counter()
    for summary in summaries:
        totals.update(summary["counts"])
    observed = {
        "passed": sum(row["status"] == "ok" for row in results),
        "failed": sum(row["status"] == "FAILED" for row in results),
        "ignored": len(ignored), "measured": 0,
    }
    if dict(totals) != observed:
        errors.append("summaries disagree with all actual test result lines")
    return {
        "executed_cases": results, "executed_count": len(results),
        "passed_count": observed["passed"], "missing": missing, "extra": extra,
        "duplicates": duplicates, "ignored": ignored, "errors": errors,
        "evidence_complete": bool(results) and not (
            missing or extra or duplicates or ignored or errors
        ),
        "summaries": summaries,
        "selection_policy": "Six independently exact Cargo calls; every result line accounted.",
    }


def check_source(source: Path, inventory: dict) -> dict:
    expected = inventory["source"]
    observed = {
        "head": text_git(source, "rev-parse", "HEAD"),
        "tree": text_git(source, "rev-parse", "HEAD^{tree}"),
        "tracked_status": text_git(source, "status", "--porcelain", "--untracked-files=no"),
        "unmerged": text_git(source, "ls-files", "-u"),
        "index_stage_sha256": hashlib.sha256(git(source, "ls-files", "--stage")).hexdigest(),
    }
    if observed["head"] != expected["head"] or observed["tree"] != expected["tree"]:
        raise ValueError("HEAD/tree differs from refreshed inventory")
    if observed["tracked_status"] or observed["unmerged"]:
        raise ValueError("source/index is not frozen clean")
    pending_merge = subprocess.run(["git", "rev-parse", "--verify", "MERGE_HEAD"], cwd=source, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    if pending_merge.returncode == 0:
        raise ValueError("final source still has a merge in progress")
    released_paths = {
        path for path in text_git(source, "ls-tree", "-r", "--name-only", observed["tree"]).splitlines()
        if re.fullmatch(r"migrations/00(?:0[1-9]|[12][0-9]|3[0-6])_.*\.sql", path)
    }
    if released_paths != set(CURRENT_MAIN_MIGRATIONS):
        raise ValueError("released Main1-36 migration paths differ")
    fingerprints = {}
    for relative, value in inventory["source_files"].items():
        path = (source / relative).resolve()
        if not path.is_relative_to(source) or not path.is_file():
            raise ValueError(f"invalid source fingerprint path: {relative}")
        actual = sha256(path)
        blob = text_git(source, "rev-parse", observed["tree"] + ":" + relative)
        if actual != value["sha256"] or blob != value["git_blob_id"]:
            raise ValueError(f"source/blob fingerprint mismatch: {relative}")
        fingerprints[relative] = actual
    for row in inventory["expected_tests"]:
        if fingerprints.get(row["source_path"]) != row["source_file_sha256"]:
            raise ValueError(f"row/file fingerprint mismatch: {pair(row)}")
    config = tomllib.loads((source / ".config/nextest.toml").read_text())
    if config["test-groups"]["automatic-envelope"]["max-threads"] != 1:
        raise ValueError("automatic-envelope max-threads changed")
    overrides = config["profile"]["default"]["overrides"]
    protected = [
        row for row in overrides if row.get("test-group") == "automatic-envelope"
        and row.get("threads-required") == "num-test-threads" and row.get("priority") == 100
        and "maximum_escaped_body_completes_and_recovers_exactly" in row.get("filter", "")
        and "actual_admission_boundary_completes_and_next_metadata_size_has_zero_effects" in row.get("filter", "")
    ]
    if len(protected) != 1:
        raise ValueError("both ALLSLOTS positives must retain the shared exact scheduling override")
    recovery = source / "tests/cli/lazy_recovery_context.rs"
    text = recovery.read_text()
    if "Duration::from_secs(30)" not in text or "Duration::from_millis(20)" not in text:
        raise ValueError("mandatory recovery30s/20ms projection waits changed")
    if sha256(recovery) != inventory["mandatory_recovery"]["source_file_sha256"]:
        raise ValueError("mandatory recovery file fingerprint changed")
    observed["source_file_sha256"] = fingerprints
    return observed


def binary_stamp(selection: dict, target: Path, required_executable: bool = True) -> dict:
    binaries = []
    for row in selection["listed_binaries"]:
        path = Path(row["path"]).resolve()
        if not path.is_relative_to(target) or not path.is_file():
            raise ValueError(f"binary lies outside the sole warmed target or is missing: {path}")
        binaries.append({**row, "sha256": sha256(path), "bytes": path.stat().st_size})
    executable = target / "debug" / "herdr-threads"
    if required_executable and not executable.is_file():
        raise ValueError("required built CLI executable is missing")
    return {
        "binaries": binaries,
        "executable": {"path": str(executable), "sha256": sha256(executable), "bytes": executable.stat().st_size}
        if executable.is_file() else None,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--selection-seal", type=Path, required=True)
    parser.add_argument("--evidence-dir", "--evidence", dest="evidence_dir", type=Path, required=True)
    parser.add_argument("--run", action="store_true")
    args = parser.parse_args()
    source = args.source.resolve()
    inventory_path = args.inventory.resolve()
    inventory = json.loads(inventory_path.read_text())
    seal_path = args.selection_seal.resolve()
    seal = json.loads(seal_path.read_text())
    seal_hash = sha256(seal_path)
    baseline_path = Path(__file__).with_name("resume-composed-matrix-preparation-f330bdab.json")
    if sha256(baseline_path) != BASELINE_SHA256:
        parser.error("immutable f330391 baseline snapshot changed")
    baseline = json.loads(baseline_path.read_text())
    historical657_path = Path(__file__).with_name(HISTORICAL657_NAME).resolve()
    historical657_bytes = historical657_path.read_bytes()
    if hashlib.sha256(historical657_bytes).hexdigest() != HISTORICAL657_SHA256:
        parser.error("immutable expanded657 inventory changed")
    historical657 = json.loads(historical657_bytes)
    if source != Path(inventory["source"]["path"]).resolve():
        parser.error("--source does not match inventory source")
    focused, mandatory = validate_inventory(inventory, baseline, seal)
    validate_historical657(inventory, seal, historical657, historical657_path)
    validate_current_main(inventory, seal)
    validate_feature_floor(inventory, seal)
    if not args.evidence_dir.is_absolute():
        parser.error("evidence directory must be absolute and fresh")
    evidence = args.evidence_dir.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    target = Path(inventory["parent_runner_requirements"]["cargo_target_dir"]).resolve()
    baseline_target = Path(baseline["parent_runner_requirements"]["cargo_target_dir"]).resolve()
    if target != baseline_target:
        parser.error("inventory must use the existing sole absolute warmed D target")
    run_id = str(uuid.uuid4())
    inventory_hash = sha256(inventory_path)
    record = {
        "schema": 1, "status": "started", "started_utc": datetime.now(timezone.utc).isoformat(),
        "run_requested": args.run, "focused_executed": False, "mandatory_executed": False,
        "source": str(source), "expected_head": inventory["source"]["head"],
        "expected_tree": inventory["source"]["tree"], "inventory": str(inventory_path),
        "inventory_sha256": inventory_hash, "selection_seal": str(seal_path),
        "selection_seal_sha256": seal_hash, "baseline_inventory": str(baseline_path),
        "baseline_sha256": BASELINE_SHA256, "runner_sha256": sha256(Path(__file__)),
        "historical657_inventory": str(historical657_path),
        "historical657_sha256": HISTORICAL657_SHA256,
        "ht_leak_run_id": run_id, "target": str(target),
        "expected_counts": {"all": len(focused) + 6, "focused": len(focused), "mandatory": 6},
        "commands": [], "artifact_limits": {"log_bytes": LOG_LIMIT, "list_bytes": LIST_LIMIT, "record_bytes": RECORD_LIMIT},
    }
    record_path = evidence / "execution.json"
    env = dict(os.environ)
    original_home = env.get("HOME")
    if original_home:
        env.setdefault("CARGO_HOME", str(Path(original_home) / ".cargo"))
        env.setdefault("RUSTUP_HOME", str(Path(original_home) / ".rustup"))
    for variable, suffix in {
        "HOME": "home", "CLAUDE_CONFIG_DIR": "claude", "CODEX_HOME": "codex",
        "GEMINI_CONFIG_DIR": "gemini", "GEMINI_CLI_HOME": "gemini-home",
    }.items():
        path = evidence / "isolated-config" / suffix
        path.mkdir(parents=True, exist_ok=True)
        env[variable] = str(path)
    env.update(CARGO_BUILD_JOBS="4", CARGO_TARGET_DIR=str(target), HT_LEAK_RUN_ID=run_id,
               HT_ACTIVATION_CAPTURE_PATH=str(evidence / "watch-activation.jsonl"),
               CARGO_TERM_COLOR="never")
    record["child_environment"] = {
        key: env[key] for key in [
            "CARGO_BUILD_JOBS", "CARGO_TARGET_DIR", "HT_LEAK_RUN_ID", "HT_ACTIVATION_CAPTURE_PATH",
            "HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "GEMINI_CONFIG_DIR", "GEMINI_CLI_HOME",
        ]
    }
    record["toolchain_environment"] = {key: env[key] for key in ("CARGO_HOME", "RUSTUP_HOME") if key in env}
    record["parent_home_modified"] = False
    save(record_path, record)
    leak_required = False
    outcome = 1
    cancellation = Cancellation()

    def command(stage: str, argv: list[str], list_json: bool = False) -> int:
        output = evidence / (stage + (".json" if list_json else ".log"))
        error_path = evidence / (stage + ".stderr.log") if list_json else None
        entry = {"stage": stage, "argv": argv, "cwd": str(source), "stdout": str(output)}
        if error_path:
            entry["stderr"] = str(error_path)
        record["commands"].append(entry)
        save(record_path, record)
        print(f"{stage}: {output}", flush=True)
        started = time.monotonic()
        child = None
        streams = []
        selector = selectors.DefaultSelector()
        errors = []
        ended_at = None
        try:
            stdout_file = output.open("wb")
            streams.append(stdout_file)
            stderr_file = error_path.open("wb") if error_path else None
            if stderr_file:
                streams.append(stderr_file)
            # Keep a created group owned even if termination arrives while
            # Popen waits for exec or immediately before assignment completes.
            with cancellation.defer():
                child = subprocess.Popen(argv, cwd=source, env=env, stdout=subprocess.PIPE,
                                         stderr=subprocess.PIPE if list_json else subprocess.STDOUT,
                                         start_new_session=True)
            selector.register(child.stdout, selectors.EVENT_READ, [stdout_file, LIST_LIMIT if list_json else LOG_LIMIT, 0, False])
            if child.stderr is not None:
                selector.register(child.stderr, selectors.EVENT_READ, [stderr_file, LOG_LIMIT, 0, False])
            while selector.get_map():
                for key, _ in selector.select(timeout=0.2):
                    chunk = os.read(key.fd, 64 * 1024)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        key.fileobj.close()
                        continue
                    file, limit, count, overflow = key.data
                    remaining = max(0, limit - count)
                    file.write(chunk[:remaining])
                    key.data[2] += len(chunk)
                    if len(chunk) > remaining and not overflow:
                        key.data[3] = True
                        errors.append(f"output exceeded{limit} bytes: {file.name}; excess drained, never treated as complete evidence")
                if child.poll() is not None:
                    ended_at = ended_at or time.monotonic()
                    if selector.get_map() and time.monotonic() - ended_at > 2:
                        errors.append("owned command exited but inherited output pipe remained open; scoped leak check required")
                        break
            actual_exit = child.wait()
            entry["exit"] = actual_exit
            if errors:
                entry["capture_errors"] = errors
            return actual_exit if actual_exit else int(bool(errors))
        finally:
            with cancellation.defer():
                if child is not None and (
                    child.poll() is None or cancellation.signum is not None
                ):
                    terminate_owned_group(child)
                    entry["interrupted_child_exit"] = child.returncode
                    entry["owned_process_group"] = child.pid
                for key in list(selector.get_map().values()):
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                selector.close()
                for stream in streams:
                    stream.close()
                entry["seconds"] = time.monotonic() - started
                for label, path in (("stdout", output), ("stderr", error_path)):
                    if path is not None and path.exists():
                        entry[label + "_sha256"] = sha256(path)
                        entry[label + "_bytes"] = path.stat().st_size
                save(record_path, record)

    def frozen(stage: str, original: dict | None = None) -> dict:
        if sha256(inventory_path) != inventory_hash or sha256(seal_path) != seal_hash:
            raise ValueError("inventory/selection seal changed during parent sweep")
        if sha256(historical657_path) != HISTORICAL657_SHA256:
            raise ValueError("expanded657 inventory changed during parent sweep")
        observed = check_source(source, inventory)
        if original is not None and observed != original:
            raise ValueError(f"source/index changed during{stage}")
        record[stage] = observed
        save(record_path, record)
        return observed

    def leak_check(stage: str) -> int:
        with cancellation.defer():
            code = command(stage, ["scripts/check-no-leaked-processes", "--run-id", run_id])
            record[stage + "_exit"] = code
        return code

    previous_signals = {sig: signal.signal(sig, cancellation.handle) for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        validate_execution_seal(inventory, seal)
        before = frozen("source_before")
        leak_required = True
        all_rows = sorted(focused + mandatory, key=pair)
        full_expression = " | ".join(predicate(row) for row in all_rows)
        focused_expression = " | ".join(predicate(row) for row in focused)
        save(evidence / "selection-expressions.json", {
            "all": full_expression, "focused": focused_expression,
            "all_sha256": hashlib.sha256(full_expression.encode()).hexdigest(),
            "focused_sha256": hashlib.sha256(focused_expression.encode()).hexdigest(),
        })
        list_argv = ["nice", "cargo", "nextest", "list", "--locked", "--all-features",
                     *TARGET_ARGS, "--message-format", "json", "-E", full_expression]
        listed = command("nextest-list", list_argv, list_json=True)
        if listed:
            raise RuntimeError(f"nextest enumeration failed or incomplete capture: exit{listed}")
        selection = inspect_selection(json.loads((evidence / "nextest-list.json").read_text()), all_rows)
        save(evidence / "run-selection.json", selection)
        record["selection"] = {
            "status": selection["status"], "selected_count": selection["selected_count"],
            "selected_by_target": selection["selected_by_target"],
            "path": str(evidence / "run-selection.json"), "sha256": sha256(evidence / "run-selection.json"),
        }
        if selection["status"] != "matched":
            raise ValueError("exact selected names rejected; inspect run-selection.json")
        frozen("source_after_list", before)
        binaries = binary_stamp(selection, target)
        record["binaries_after_list"] = binaries
        record["status"] = "enumerated_not_run"
        outcome = 0
        if args.run:
            focused_code = 1
            focused_leaks = 1
            frozen("source_before_focused", before)
            if binary_stamp(selection, target) != binaries:
                raise ValueError("binaries/executable changed before focused run")
            record["binaries_before_focused"] = binaries
            try:
                record["focused_executed"] = True
                focused_code = command("nextest-run", [
                    "nice", "cargo", "nextest", "run", "--locked", "--all-features",
                    *TARGET_ARGS, "--status-level", "pass", "--final-status-level", "none",
                    "--no-fail-fast", "-E", focused_expression,
                ])
                results = parse_nextest_results(evidence / "nextest-run.log", focused)
                save(evidence / "focused-results.json", results)
                record["focused_results"] = results
                frozen("source_after_focused", before)
                record["binaries_after_focused"] = binary_stamp(selection, target)
                if record["binaries_after_focused"] != binaries:
                    raise ValueError("focused binary/executable hashes changed")
                if not results["evidence_complete"] or results["passed_count"] != len(focused):
                    focused_code = focused_code or 1
            finally:
                focused_leaks = leak_check("focused-scoped-leaks")
            if focused_leaks:
                raise RuntimeError("focused scoped leaks remain; mandatory stage not started")
            frozen("source_before_mandatory", before)
            record["binaries_before_mandatory"] = binary_stamp(selection, target)
            if record["binaries_before_mandatory"] != binaries:
                raise ValueError("binaries/executable changed before mandatory stage")
            mandatory_code = 1
            mandatory_leaks = 1
            try:
                record["mandatory_executed"] = True
                mandatory_code = 0
                mandatory_logs = []
                record["mandatory_exact_cases"] = []
                for index, row in enumerate(mandatory, 1):
                    stage = f"mandatory-recovery-{index:02}"
                    frozen(f"source_before_{stage}", before)
                    if binary_stamp(selection, target) != binaries:
                        raise ValueError("mandatory exact-case binaries changed")
                    case_exit = command(stage, mandatory_argv(row))
                    log = evidence / (stage + ".log")
                    case_results = parse_recovery_results(log, [row])
                    record["mandatory_exact_cases"].append({
                        "binary_id": row["binary_id"], "name": row["name"],
                        "exit": case_exit, "results": case_results,
                    })
                    mandatory_logs.append(log)
                    if case_exit or not case_results["evidence_complete"] or case_results["passed_count"] != 1:
                        mandatory_code = 1
                    frozen(f"source_after_{stage}", before)
                    if binary_stamp(selection, target) != binaries:
                        raise ValueError("mandatory exact-case binaries changed")
                results = parse_recovery_results(mandatory_logs, mandatory)
                save(evidence / "mandatory-results.json", results)
                record["mandatory_results"] = results
                frozen("source_after_mandatory", before)
                record["binaries_after_mandatory"] = binary_stamp(selection, target)
                if record["binaries_after_mandatory"] != binaries:
                    raise ValueError("mandatory stage binary/executable hashes changed")
                if not results["evidence_complete"] or results["ignored"] or results["passed_count"] != 6:
                    mandatory_code = mandatory_code or 1
            finally:
                mandatory_leaks = leak_check("mandatory-scoped-leaks")
            outcome = int(bool(focused_code or mandatory_code or mandatory_leaks))
            record["status"] = "run_failed" if outcome else "run_passed_cleanup_pending"
    except TerminationRequested as error:
        record["status"] = "interrupted"
        record["termination_signal"] = error.signum
        record["error"] = "Catchable termination; owned group cleanup and scoped scans attempted."
        outcome = 128 + error.signum
    except KeyboardInterrupt:
        record["status"] = "interrupted"
        record["error"] = "Parent interruption; only runner-owned command group stopped; scoped cleanup still attempted."
        outcome = 130
    except Exception as error:
        record["status"] = "failed"
        record["error"] = f"{type(error).__name__}: {error}"
        outcome = 1
    finally:
        with cancellation.defer(raise_after=False):
            if leak_required:
                try:
                    leak_exit = leak_check("final-scoped-leaks")
                    if leak_exit:
                        record["status_before_cleanup_failure"] = record["status"]
                        record["status"] = "cleanup_failed"
                        outcome = 1
                    elif record["status"] == "run_passed_cleanup_pending":
                        record["status"] = "passed"
                except Exception as error:
                    record["cleanup_error"] = f"{type(error).__name__}: {error}"
                    record["status"] = "cleanup_failed"
                    outcome = 1
            try:
                record["source_final"] = check_source(source, inventory)
                if "source_before" in record and record["source_final"] != record["source_before"]:
                    raise ValueError("final source/index differs")
                if sha256(inventory_path) != inventory_hash or sha256(seal_path) != seal_hash:
                    raise ValueError("final inventory/selection seal differs")
                if "binaries_after_list" in record:
                    record["binaries_final"] = binary_stamp(selection, target)
                    if record["binaries_final"] != record["binaries_after_list"]:
                        raise ValueError("final binary/executable differs")
            except Exception as error:
                record["final_provenance_error"] = f"{type(error).__name__}: {error}"
                record["status"] = "provenance_failed"
                outcome = outcome or 1
            capture = Path(env["HT_ACTIVATION_CAPTURE_PATH"])
            if capture.is_file():
                record["watch_capture"] = {"path": str(capture), "sha256": sha256(capture), "bytes": capture.stat().st_size}
                if capture.stat().st_size > LOG_LIMIT:
                    record["watch_capture_limit_warning"] = "External producer capture exceeds log budget; retained honestly, never truncated by this runner."
                    record["status"] = "capture_limit_exceeded"
                    outcome = outcome or 1
            if cancellation.signum is not None:
                record["termination_signal"] = cancellation.signum
                record["status_before_termination"] = record["status"]
                record["status"] = "interrupted"
                outcome = 128 + cancellation.signum
            record["finished_utc"] = datetime.now(timezone.utc).isoformat()
            record["exit"] = outcome
            save(record_path, record)
        for sig, previous in previous_signals.items():
            signal.signal(sig, previous)
    print(json.dumps({"status": record["status"], "evidence": str(record_path), "run_id": run_id}), flush=True)
    return outcome


if __name__ == "__main__":
    sys.exit(main())
