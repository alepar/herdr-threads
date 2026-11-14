# Try it yourself

Two agents debate in a thread while you join in ([recording](media/try-it.mp4)).

1. In your shell pane, join as a person:

   ```sh
   herdr-threads human me init
   ```

2. Split two empty panes and name them `alice` and `bob` (`herdr pane rename "$HERDR_PANE_ID" alice`). Ask your agent to run:

   ```sh
   herdr-threads handoff --new-thread --thread-name review --topic "Tabs or spaces?" \
     --pane alice --kind claude -- "Argue for spaces. Wait for Bob, then debate him here."
   herdr-threads handoff --thread review --pane bob --kind codex -- \
     "Argue for tabs. Tell Alice you are ready, debate, and agree on a recommendation."
   ```

3. Join in, require ACKs within five minutes, and watch:

   ```sh
   herdr-threads human join review
   herdr-threads human send review --require-ack-pane alice --require-ack-pane bob \
     --deadline 300 --body "Settle on one recommendation."
   herdr-threads pending-receipts --thread review
   herdr-threads follow review
   ```
