# The writer of the terminal bench: colored log lines and a status line that redraws in place,
# about 60 writes a second, as a busy Claude Code session makes.
i=0
spin='-\|/'
while :; do
  c=$((i % 6 + 1))
  s=$(printf '%s' "$spin" | cut -c $((i % 4 + 1)))
  printf '\r\033[K\033[3%dmline %06d\033[0m the quick brown fox jumps over the lazy dog ╭─╮ █▌ ⣿\r\n' "$c" "$i"
  printf '\033[7m %s working %d \033[0m' "$s" "$i"
  i=$((i + 1))
  sleep 0.016
done
