# git-parsec Demo
Output demo.gif
Output demo.mp4

Set Shell "zsh"
Set FontSize 15
Set Width 900
Set Height 500
Set Theme "Catppuccin Mocha"
Set TypingSpeed 50ms
Set Padding 20

# Setup: create a temp repo
Hide
Type `export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"`
Enter
Sleep 500ms
Type "cd /tmp && rm -rf parsec-demo && mkdir parsec-demo && cd parsec-demo"
Enter
Sleep 500ms
Type "git init && git commit --allow-empty -m 'init'"
Enter
Sleep 500ms
Show

# Title
Type "# git-parsec: Worktree Lifecycle Manager"
Sleep 1s
Enter
Sleep 500ms

# 1. Start a ticket
Type "parsec start 42 --title 'Add user authentication'"
Sleep 500ms
Enter
Sleep 2s

# 2. Start another ticket (parallel work!)
Type "parsec start 99 --title 'Fix payment timeout bug'"
Sleep 500ms
Enter
Sleep 2s

# 3. List active worktrees
Type "parsec list"
Sleep 500ms
Enter
Sleep 3s

# 4. Check status
Type "parsec status 42"
Sleep 500ms
Enter
Sleep 3s

# 5. Check conflicts
Type "parsec conflicts"
Sleep 500ms
Enter
Sleep 2s

# 6. Switch to worktree
Type "cd $(parsec switch 42)"
Sleep 500ms
Enter
Sleep 1s

# 7. Show where we are
Type "pwd"
Sleep 300ms
Enter
Sleep 2s

# 8. Clean up
Type "cd /tmp/parsec-demo"
Enter
Sleep 500ms
Type "parsec clean --dry-run"
Sleep 500ms
Enter
Sleep 2s

# 9. Clean all
Type "parsec clean --all"
Sleep 500ms
Enter
Sleep 2s

Type "# Done! Install: cargo install git-parsec"
Sleep 2s
Enter
Sleep 3s
