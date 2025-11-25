# Testing

This document provides a regression testing checklist for COSMIC Edit. The checklist provides a starting point for Quality Assurance reviews.

## Checklist:

### Basic operations

- [ ] Type three lines of text (no trailing newline).
- [ ] Copy the last two lines.
- [ ] Paste at the end of the third line (file should now have four lines).
- [ ] Ctrl-Z to undo the paste.
- [ ] Press Enter to add a trailing newline.
- [ ] Paste again (file should now have five lines).
- [ ] Ctrl-Z to undo the paste.
- [ ] Ctrl-Shift-Z to redo the paste.
- [ ] Save the file.
- [ ] Ctrl-F and search for something that has a match.
- [ ] Press Esc twice to exit the Find dialog.
- [ ] Press Ctrl-X to cut the selected search result.
- [ ] Paste the cut text on a new line (file should now have six lines).
- [ ] Re-save the file.
- [ ] Narrow the window until the lines start wrapping (make a line longer if necessary to observe line wrapping).
- [ ] Turn word wrapping off.
- [ ] Scroll right to the end of the document, then left to the beginning again.
- [ ] Click and drag to select some text past the horizontal edge of the window.
- [ ] Close the file, open COSMIC Edit again, and open the file via the recents list.
- [ ] Close the file again, open COSMIC Edit again, and open the file via the Open dialog.
- [ ] Turn word wrapping back on.

### Settings

- [ ] Open View -> Settings.
- [ ] All Appearance settings work.
- [ ] Vim bindings work.

### Projects & Git Management

- [ ] Clone the cosmic-edit Git repo and open its directory as a project.
- [ ] Edit -> Find in project... works.
- [ ] Make a change in a file.
- [ ] File -> Git management shows the change and staging it works.
- [ ] Make another change while Git management's open; it updates to show the new change.
