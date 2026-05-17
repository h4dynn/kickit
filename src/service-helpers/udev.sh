#!/bin/bash -euf

# Start udev daemon in background
udevd &

# Trigger subsystems & devices
udevadm trigger --action=add --type=subsystems
udevadm trigger --action=add --type=devices
udevadm settle

# For as long as udevd is running, we don't exit
wait
