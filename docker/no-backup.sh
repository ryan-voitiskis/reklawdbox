#!/bin/sh
# Pre-export backup hook for the container (REKLAWDBOX_BACKUP_SCRIPT).
#
# write_xml normally backs up master.db before exporting. In the container the
# database is a disposable copy of a snapshot, and the real library on the
# Rekordbox host is never written: changes reach it only when the exported
# XML is imported there. Backing up the copy would only fill the disk.
exit 0
