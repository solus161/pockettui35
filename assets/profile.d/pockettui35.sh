# pockettui35: start the launcher after the autologin on tty1.
# Installed as /etc/profile.d/pockettui35.sh; @USER@ is the session user.
# Other ttys, SSH, other users, and the maintenance shell are left alone.
if [ "$(tty)" = /dev/tty1 ] && [ "${USER:-}" = "@USER@" ] \
  && [ -z "${PT35_NO_LAUNCHER:-}" ] && command -v pockettui35-session >/dev/null; then
  exec pockettui35-session
fi
