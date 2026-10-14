export default {
  "items": [
    {
      "kind": "message",
      "id": "m-p5",
      "thread": "T1",
      "thread_name": "plans",
      "sender": "S1",
      "sender_name": "alice",
      "priority": 5,
      "wakes": true,
      "ack_required": false,
      "body": "background note"
    },
    {
      "kind": "message",
      "id": "m-p1",
      "thread": "T1",
      "thread_name": "plans",
      "sender": "S1",
      "sender_name": "alice",
      "priority": 1,
      "wakes": true,
      "ack_required": false,
      "body": "ordinary note"
    },
    {
      "kind": "invitation",
      "id": "invitation:i-1",
      "thread": "T2",
      "thread_name": "invited",
      "sender": "S1",
      "sender_name": "alice",
      "priority": 0,
      "deadline_at": 600000,
      "wakes": true,
      "ack_required": true,
      "body": "join"
    },
    {
      "kind": "message",
      "id": "m-p0",
      "thread": "T1",
      "thread_name": "plans",
      "sender": "S1",
      "sender_name": "alice",
      "priority": 0,
      "wakes": true,
      "ack_required": false,
      "body": "urgent note"
    },
    {
      "kind": "message",
      "id": "m-ack",
      "thread": "T1",
      "thread_name": "plans",
      "sender": "S1",
      "sender_name": "alice",
      "priority": 0,
      "deadline_at": 120000,
      "wakes": true,
      "ack_required": true,
      "body": "please ack"
    }
  ],
  "expected": [
    "m-ack",
    "invitation:i-1",
    "m-p0",
    "m-p1",
    "m-p5"
  ]
}
