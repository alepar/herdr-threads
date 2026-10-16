# super-auto run — 2026-10-15-inbox-injection-auto-ack

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: design
codeMechanism: ordinary-subagents

idea: hooks/channels only direct to call inbox when directly injected messages doesnt cover entire inbox, ie there are more. agent should never need to call inbox explicitly, unless there's a spike of messages received and it overflows the size that hook/channel can inject in a single turn. messages injected by hooks/channels or read via inbox do generate automatic receipt ack. normally, agent should never have the need to call explicit ack cli command. let's fix via $super-auto and communicate to p2p tab (it might care about the second ruling specifically, make sure this auto ack works scross multiple nodes too). fully autonomous, both roasts on.
branch: super-auto/inbox-injection-auto-ack
base: main
