<!-- Shortened version of README.md for rustdoc -->

![](https://github.com/h4dynn/kickit/raw/refs/heads/main/assets/logo.svg)

![Lint](https://github.com/h4dynn/kickit/actions/workflows/lint.yml/badge.svg)
![Nightly Lint](https://github.com/h4dynn/kickit/actions/workflows/nightly-lint.yml/badge.svg)

## Why?

### Immutability

The traditional design of an init system, is that you can kill/restart
a service whenever you wish. This is great for power users or easy &
quick system tinkering, however it could also pose a security risk.

Say, for example, you have a service for your display manager running,
a malicious script/program ran as the root user, could just kill it
and all of a sudden you have no access to your desktop.

This is a very niche example, but also could totally happen. If you're
a security freak like me, then you might like the sound of this project.

### Sandboxing

`kickit` comes with its own sandboxer, called `warden`. It is directly
inspired by the namespace sandboxer
[bubblewrap](https://github.com/containers/bubblewrap) which is used
in `flatpak`.

The rationale behind sandboxing is that a program should really not
be able to access anything more than it needs to. This is for both
security & privacy. From a security perspective, imagine a program
has an exploit, abused by another malicious program. Because this
program is unsandboxed, the malicious program potentially now has
access to not just what the original program was designed to do,
but also other resources. Sandboxing does not stop these exploits
at all, but it does mitigate some additional security risks.

Additionally, services running at the init level usually are given
root access, which opens the biggest security hole. Sandboxing won't
completely address this, but may help to mitigate any CVEs (though
this is purely hypothetical, not backed up by any real life examples)

## Design

`kickit` does not aim to be a replacement for any existing init
system, it intends to be its own thing entirely.

It can be ran as the init, or alongside another init system, like
`systemd` or really any other as long as you make a service for it!

I highly recommend if you are going to try this thing out, to try
it alongside your existing init system before using as the init.
This will help you to get a feel for it, and see if this is really
what you want or just another piece of crapware from GitHub.
