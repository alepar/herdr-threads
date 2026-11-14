# Recording the Quick start video

[quick-start.mp4](quick-start.mp4) shows the README's channel example: Claude Code in the top pane, a shell below, and the tab bar filling as five tabs start. The agents and their messages are real. Waits are sped up; typing plays at recorded speed.

Everything runs in a private rig: its own Herdr server, herdr-threads daemon and state directory, and a scratch `HOME` with hooks installed by `herdr-threads setup`. The shared Herdr server, the real daemon and the real harness config are never touched. Claude gets its credential from `CLAUDE_CODE_OAUTH_TOKEN`, read from a mode-0600 file; the take spends real quota (six Claude sessions for about two minutes).

## Record

Needs `herdr`, `herdr-threads` and `claude` on `PATH`, and for rendering [agg](https://github.com/asciinema/agg) and `ffmpeg`.

```sh
D=scripts/demo-quick-start
R=$(python3 $D/rig.py up TOKEN_FILE)                      # prints the rig root, /tmp/hqs.*
H="python3 $D/rig.py env $R --"
$H herdr workspace create --label demo --cwd $R/home/demo --no-focus
$H herdr tab rename w1:t1 main
$H herdr pane split w1:p1 --direction down --ratio 0.74 --cwd $R/home/demo
$H herdr pane run w1:p1 "clear; claude"                   # wait for the composer; if a first-run notice shows, /exit and start it again
$H herdr pane run w1:p2 clear
(cd $R/home/demo && $H python3 $OLDPWD/$D/cam.py $R --cols 100 --rows 34) &   # the recording client
(cd $R/home/demo && $H python3 $OLDPWD/$D/take.py $R)     # types the four prompts, then stops the camera
cp $R/camera.cast take.cast
python3 $D/rig.py down $R
```

`take.py` types each prompt through the recording client, waits for the agents, enlarges the shell pane and runs `herdr-threads follow weather`. What the agents do varies from take to take: read every pane and retake until all five tabs post.

## Render

```sh
python3 scripts/demo-quick-start/warp.py take.cast warped.cast   # compress the waits between prompts
agg --theme dracula --font-family "Adwaita Mono" --font-size 17 --line-height 1.25 \
  --fps-cap 15 --idle-time-limit 600 --last-frame-duration 3 warped.cast take.gif
ffmpeg -i take.gif -an -c:v libx264 -crf 24 -vf 'fps=15,pad=ceil(iw/2)*2:ceil(ih/2)*2' \
  -pix_fmt yuv420p -movflags +faststart docs/media/quick-start.mp4
```

The font must cover the harness's glyphs (`❯`, `✻`, `⏵`, arrows); a missing glyph renders as a box.

Before committing, extract stills (`ffmpeg -i quick-start.mp4 -vf fps=1/3,scale=640:-1,tile=3x4 sheet.png`) and check each for legibility at README width and for private data: home paths, host names, real workspaces, tokens.

## 2026-11-14 take

Claude Code 2.1.325 (Sonnet 5.5), Herdr 0.9.5, herdr-threads 3.1.1. 1040×744, 15 fps, 59 s, 1,580,315 bytes, SHA-256 `f530a3f24a95feac05929472f0d56f225b1cf3af5198232e2191da660d366980`. Raw take 134 s; the waits after each prompt play at 1.6× to 12.5×. It is the fourth of five takes; the others had fewer tabs posting or a worse layout.
