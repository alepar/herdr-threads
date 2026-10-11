export default {
  "header": "[herdr-threads] new messages:",
  "empty": {
    "items": [],
    "text": "[herdr-threads] new messages:\n\n"
  },
  "multiline": {
    "items": [
      {
        "kind": "message",
        "id": "m1",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "first\r\nsecond third\n\nfifth after an empty line\rsixth\u000bseventh\feighthninth tenth"
      },
      {
        "kind": "lazy",
        "id": "m2",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": ""
      },
      {
        "kind": "message",
        "id": "m3",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "trailing break\n"
      }
    ],
    "text": "[herdr-threads] new messages:\n\n[herdr-threads] message m1 in T1 \"plans\" from S1 \"alice\":\n  first\n  second\n  third\n\n  fifth after an empty line\n  sixth\n  seventh\n  eighth\n  ninth\n  tenth\n\n[herdr-threads] lazy m2 in T1 \"plans\" from S1 \"alice\":\n\n\n[herdr-threads] message m3 in T1 \"plans\" from S1 \"alice\":\n  trailing break\n"
  },
  "controls": {
    "items": [
      {
        "kind": "message",
        "id": "m1",
        "thread": "T\u0001\u001f1",
        "threadName": "pl\tans\n\"quoted\" \\ back slash",
        "sender": "S\r\n1",
        "senderName": "al\u0000ice  \"x\"",
        "body": "ok"
      },
      {
        "kind": "mes\nsage",
        "id": "m\u00072",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "ok"
      }
    ],
    "text": "[herdr-threads] new messages:\n\n[herdr-threads] message m1 in T  1   \"pl ans \\\"quoted\\\" \\\\ back slash\" from S  1 \"al ice  \\\"x\\\"\":\n  ok\n\n[herdr-threads] mes sage m 2 in T1 \"plans\" from S1 \"alice\":\n  ok"
  },
  "name-equals-id": {
    "items": [
      {
        "kind": "message",
        "id": "m1",
        "thread": "T1",
        "threadName": "T1",
        "sender": "S1",
        "senderName": "S1",
        "body": "body of m1"
      },
      {
        "kind": "message",
        "id": "m2",
        "thread": "T1",
        "threadName": "T1\n",
        "sender": "S1",
        "senderName": "S1",
        "body": "body of m2"
      },
      {
        "kind": "message",
        "id": "m3",
        "thread": "",
        "threadName": "only a name",
        "sender": "",
        "senderName": "",
        "body": "body of m3"
      },
      {
        "kind": "message",
        "id": "m4",
        "thread": "",
        "threadName": "",
        "sender": "",
        "senderName": "",
        "body": "body of m4"
      }
    ],
    "text": "[herdr-threads] new messages:\n\n[herdr-threads] message m1 in T1 from S1:\n  body of m1\n\n[herdr-threads] message m2 in T1 \"T1 \" from S1:\n  body of m2\n\n[herdr-threads] message m3 in \"only a name\" from unknown:\n  body of m3\n\n[herdr-threads] message m4 in unknown from unknown:\n  body of m4"
  },
  "markers": {
    "items": [
      {
        "kind": "message",
        "id": "m1",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "human author",
        "author_role": "human"
      },
      {
        "kind": "message",
        "id": "m2",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "relays the user",
        "relays_user": true
      },
      {
        "kind": "message",
        "id": "m3",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "query",
        "user_intent": "query"
      },
      {
        "kind": "message",
        "id": "m4",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "request",
        "user_intent": "request"
      },
      {
        "kind": "message",
        "id": "m5",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "rule",
        "user_intent": "rule"
      },
      {
        "kind": "message",
        "id": "m6",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "unknown intent adds no marker",
        "user_intent": "bogus"
      },
      {
        "kind": "message",
        "id": "m7",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "all markers, ack required",
        "author_role": "human",
        "relays_user": true,
        "user_intent": "rule",
        "ack_required": true
      },
      {
        "kind": "message",
        "id": "m8",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "no marker",
        "author_role": "agent",
        "relays_user": false,
        "ack_required": true
      }
    ],
    "text": "[herdr-threads] new messages:\n\n[herdr-threads] message m1 in T1 \"plans\" from S1 \"alice\" [human]:\n  human author\n\n[herdr-threads] message m2 in T1 \"plans\" from S1 \"alice\" [relays user]:\n  relays the user\n\n[herdr-threads] message m3 in T1 \"plans\" from S1 \"alice\" [query]:\n  query\n\n[herdr-threads] message m4 in T1 \"plans\" from S1 \"alice\" [request]:\n  request\n\n[herdr-threads] message m5 in T1 \"plans\" from S1 \"alice\" [rule]:\n  rule\n\n[herdr-threads] message m6 in T1 \"plans\" from S1 \"alice\":\n  unknown intent adds no marker\n\n[herdr-threads] message m7 in T1 \"plans\" from S1 \"alice\" [human] [relays user] [rule]:\n  all markers, ack required\n\n[herdr-threads] message m8 in T1 \"plans\" from S1 \"alice\":\n  no marker"
  },
  "attention": {
    "items": [
      {
        "kind": "attention",
        "id": "attention:4",
        "body": "You have pending herdr-threads items."
      },
      {
        "kind": "message",
        "id": "m1",
        "thread": "T1",
        "threadName": "plans",
        "sender": "S1",
        "senderName": "alice",
        "body": "body of m1"
      },
      {
        "kind": "attention",
        "id": "attention:5\n",
        "body": "line one\nline two"
      }
    ],
    "text": "[herdr-threads] new messages:\n\n[herdr-threads] attention attention:4:\n  You have pending herdr-threads items.\n\n[herdr-threads] message m1 in T1 \"plans\" from S1 \"alice\":\n  body of m1\n\n[herdr-threads] attention attention:5 :\n  line one\n  line two"
  }
}
