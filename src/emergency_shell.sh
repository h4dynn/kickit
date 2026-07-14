#!/bin/sh -euf
#
# Create a new emergency shell for kickit
#

# Execute the shell with custom PS1 (prompt) & start at root directory
exec -c env -S -C / PS1='\[\e[1;31m\](emergency)\[\e[0;1m\] \w \[\e[0m\]$ ' "$1" -ims;

# We shouldn't reach this point unless the user exits for some reason
exit 1;
