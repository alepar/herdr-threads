#!/bin/bash
cd /var/home/alepar/AleCode/herdr-threads/.worktrees/external-seat/.worktrees/super-auto-environments-named-seats-herdr-app--task-ht-kqrt.62
bash evidence/ht-kqrt.62/suite.sh > evidence/ht-kqrt.62/final/suite.log 2>&1
bash evidence/ht-kqrt.62/rest.sh > evidence/ht-kqrt.62/final/rest.log 2>&1
echo FINALDONE >> evidence/ht-kqrt.62/final/rest.log
