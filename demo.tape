# git-parsec Demo
Output demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 1000
Set Height 550
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Set Padding 24
Set WindowBar Colorful
Set WindowBarSize 40
Set CursorBlink false

# Hidden setup: create temp repo with proper prompt
Hide
Type `export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"`
Enter
Sleep 300ms
Type `export PS1="\[\033[1;36m\]parsec-demo\[\033[0m\] \[\033[1;33m\]→\[\033[0m\] "`
Enter
Sleep 300ms
Type "cd /tmp && rm -rf parsec-demo && mkdir parsec-demo && cd parsec-demo"
Enter
Sleep 300ms
Type "git init -q && git commit -q --allow-empty -m 'init'"
Enter
Sleep 500ms
Type "clear"
Enter
Sleep 300ms
Show

# 1. Start first ticket
Sleep 500ms
Type "parsec start 42 --title 'Add user authentication'"
Sleep 300ms
Enter
Sleep 2500ms

# 2. Start second ticket (parallel!)
Type "parsec start 99 --title 'Fix payment timeout'"
Sleep 300ms
Enter
Sleep 2500ms

# 3. List
Type "parsec list"
Sleep 300ms
Enter
Sleep 3s

# 4. Status detail
Type "parsec status 42"
Sleep 300ms
Enter
Sleep 3s

# 5. Conflicts
Type "parsec conflicts"
Sleep 300ms
Enter
Sleep 2500ms

# 6. Switch
Type "cd $(parsec switch 42) && pwd"
Sleep 300ms
Enter
Sleep 2s

# 7. Go back and clean
Type "cd /tmp/parsec-demo"
Enter
Sleep 500ms
Type "parsec clean --all"
Sleep 300ms
Enter
Sleep 2500ms

# 8. Done
Type "parsec list"
Sleep 300ms
Enter
Sleep 3s
