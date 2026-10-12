// Complete actual elected advertised Watch stdout, including the final newline.
// Expected headers are hand-reviewed literals; external ACK/engine outcomes in
// delivery.test.ts remain test doubles, not evidence of live daemon transitions.
export default {
  raw: "{\"schema\":1,\"id\":\"status:0\",\"kind\":\"status\",\"state\":\"connected\"}\n{\"schema\":1,\"id\":\"mw7dy8CUc\",\"kind\":\"lazy\",\"thread\":\"tmFxjHevw\",\"thread_name\":\"mod delivery\",\"sender\":\"seM75yOxM\",\"sender_name\":\"seM75yOxM\",\"author_role\":\"agent\",\"relays_user\":false,\"user_intent\":null,\"body\":\"activation passive lazy\",\"body_len\":23,\"truncated\":false,\"ack_required\":false}\n{\"schema\":1,\"id\":\"mbwolAHtW\",\"kind\":\"message\",\"thread\":\"tmFxjHevw\",\"thread_name\":\"mod delivery\",\"sender\":\"seM75yOxM\",\"sender_name\":\"seM75yOxM\",\"author_role\":\"agent\",\"relays_user\":false,\"user_intent\":null,\"body\":\"activation ordinary ACK candidate\",\"body_len\":33,\"truncated\":false,\"ack_required\":true}\n{\"schema\":1,\"id\":\"notice:nMJDRPQ3Z\",\"kind\":\"notice\",\"warning\":\"nMJDRPQ3Z\",\"thread\":\"activation-warn\",\"body\":\"informational, no ACK; read: herdr-threads --state-dir /private/tmp/htmd-f38b3559/state body nMJDRPQ3Z\"}\n{\"schema\":1,\"id\":\"invitation:ivRL92tt0\",\"kind\":\"invitation\",\"command_context_epoch\":\"0\",\"topic_revision\":\"0\",\"thread\":\"tncVlNIeG\",\"thread_name\":\"activation invitation\",\"body\":\"activation invitation\\ngoal: activation invitation\\nnext: herdr-threads --state-dir /private/tmp/htmd-f38b3559/state accept tncVlNIeG\",\"truncated\":false,\"wakes\":false}\n{\"schema\":1,\"id\":\"attention:1\",\"kind\":\"attention\",\"presentation_epoch\":\"1\",\"attention_version\":3006609949771651170,\"text\":\"herdr-threads: attention pending; run herdr-threads --state-dir /private/tmp/htmd-f38b3559/state inbox\"}\n{\"schema\":1,\"id\":\"attention_cleared:1\",\"kind\":\"attention_cleared\",\"presentation_epoch\":\"1\",\"attention_version\":3006609949771651170}\n",
  provenance: {
    producerHead: 'ec04361ed0fc7efbfa7118d296621a25cf5a1175',
    producerTree: 'a9f9cadfd328180904d1ec5998bfdfd8d4dfb41e',
    executionSource: 'ad9d8f176897c4ae3f390566533f75d26914b300 plus the exact one-line execution patch committed as producerHead; parent attests identical precommit dirty tree',
    producerTestSha256: '58bfd423e2d9339eca4a7bc9a66d91264f4b6f70250d7af91d543a661a5894fe',
    producerTest: 'mod_delivery::activation_v2_elected_watch_capabilities_lease_output_confirm',
    producerResult: 'PASS (3.599s); capture is written only after all positive assertions',
    executable: '/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2/target/debug/herdr-threads',
    executableSha256: 'df396e835cc58b1cff80e67a56f523f1978e494e5d114cd44d6c1e5e1e4b8596',
    executableBytes: 45039408,
    buildCorrespondence: 'parent-owned native run5 built the one-line-corrected source; current clean producerHead/tree and test/executable hashes were independently checked before fixture installation',
    capture: '/private/tmp/ht-notice-green-capture.jsonl',
    captureSha256: '596bc16bc63004d37ef41454f4e71dbb608fe3ea14d7c94a3fcb69eb27f8f53c',
    captureBytes: 1571,
    log: '/private/tmp/ht-notice-green-native.log',
    logSha256: '4dff8142828094af600620948f028231f9fb3bdafa9a8a066e2df019bd041c83',
    command: "nice cargo nextest run --locked --all-features --lib --test integration -E 'test(presentation_hook_notice_settles_current_execution_without_reoffer) | test(presentation_watch_notice_settles_current_execution_without_reoffer) | test(=mod_delivery::activation_v2_watch_lost_confirm_response_replays_exact_subset) | test(=mod_delivery::activation_v2_watch_replacement_replays_uncommitted_output) | test(=mod_delivery::activation_v2_elected_watch_capabilities_lease_output_confirm)'",
    environment: {
      CARGO_BUILD_JOBS: '4',
      CARGO_TARGET_DIR: '/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2/target',
      HT_LEAK_RUN_ID: '7078da3c-efba-4a17-b65f-44cfef854106',
      HT_ACTIVATION_CAPTURE_PATH: '/private/tmp/ht-notice-green-capture.jsonl',
    },
    selectionResult: '5 executed: 4 PASS, 1 FAIL; replacement Watch fixture failure remains open and is not cleared by this capture',
    passed: [
      'service::delivery_pull::tests::presentation_watch_notice_settles_current_execution_without_reoffer',
      'service::delivery_pull::tests::presentation_hook_notice_settles_current_execution_without_reoffer',
      'mod_delivery::activation_v2_watch_lost_confirm_response_replays_exact_subset',
      'mod_delivery::activation_v2_elected_watch_capabilities_lease_output_confirm',
    ],
    failed: ['mod_delivery::activation_v2_watch_replacement_replays_uncommitted_output'],
    scopedLeaks: 'parent-reported PASS, zero owned leaks for HT_LEAK_RUN_ID above',
    pluginExecution: 'NOT RUN; parent owns validation and plugin tests',
    evidenceBoundary: 'actual CLI producer output replayed through real mod handlers with mocked external APIs; no live native Claude child or model uptake claim',
  },
  ids: { lazy: 'mw7dy8CUc', message: 'mbwolAHtW', attention: 'attention:1' },
  passiveBodies: [
    '  activation passive lazy',
    '  informational, no ACK; read: herdr-threads --state-dir /private/tmp/htmd-f38b3559/state body nMJDRPQ3Z',
    '  activation invitation\n  goal: activation invitation\n  next: herdr-threads --state-dir /private/tmp/htmd-f38b3559/state accept tncVlNIeG',
  ],
  attentionBody: '  herdr-threads: attention pending; run herdr-threads --state-dir /private/tmp/htmd-f38b3559/state inbox',
  passiveHeaders: [
    '[herdr-threads] lazy mw7dy8CUc in tmFxjHevw "mod delivery" from seM75yOxM:',
    '[herdr-threads] notice notice:nMJDRPQ3Z in activation-warn from unknown:',
    '[herdr-threads] invitation invitation:ivRL92tt0 in tncVlNIeG "activation invitation" from unknown:',
  ],
  activeHeaders: [
    '[herdr-threads] message mbwolAHtW in tmFxjHevw "mod delivery" from seM75yOxM:',
    '[herdr-threads] attention attention:1:',
  ],
  nonmessageHeaders: [
    '[herdr-threads] notice notice:nMJDRPQ3Z in activation-warn from unknown:',
    '[herdr-threads] invitation invitation:ivRL92tt0 in tncVlNIeG "activation invitation" from unknown:',
  ],
  messageHeaders: [
    '[herdr-threads] message mbwolAHtW in tmFxjHevw "mod delivery" from seM75yOxM:',
  ],
}
