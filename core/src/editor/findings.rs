//! The findings' half of the facade: the markers that make a finding follow
//! the text of an open file, made when a producer replaces its findings or
//! when the file is opened, and read when the findings are asked for.
//!
//! Design: `doc/architettura-editor.typ, <segnalazioni>`.

use super::*;
use crate::managers::{Finding, Found, LineColumn, Placed};

/// A finding as it is asked for: where it is now, and whether its file's
/// text has changed since its producer spoke.
#[derive(Clone, Debug)]
pub(crate) struct Located {
    pub file: String,
    pub producer: String,
    pub finding: Finding,
    pub stale: bool,
}

/// Returns the offset in TEXT of a producer's line, from 1, and column,
/// both kept within the text.
fn offset_of<B: BufferTrait>(text: &B, (line, column): LineColumn) -> usize {
    let line = line
        .saturating_sub(1)
        .min(text.line_count().saturating_sub(1));
    let column = column.min(crate::primitives::movement::line_length(text, line));
    text.cursor_2d_to_1d(line, column)
}

/// Returns the producer's line, from 1, and column of OFFSET in TEXT.
fn line_column_of<B: BufferTrait>(text: &B, offset: usize) -> LineColumn {
    let (line, column) = text.cursor_1d_to_2d(offset.min(text.len()));
    (line + 1, column)
}

/// Makes a pair of markers in BUF for each of FINDINGS, and returns them with
/// the buffer's version.
fn make_markers<B: BufferTrait>(buf: &mut Buffer<B>, findings: &[Finding]) -> Placed {
    let markers = findings
        .iter()
        .map(|finding| {
            let start = offset_of(&buf.text, finding.start);
            let end = offset_of(&buf.text, finding.end).max(start);
            (buf.markers.add(start), buf.markers.add(end))
        })
        .collect();
    Placed {
        markers,
        version: buf.version,
    }
}

impl<B: BufferTrait> EditorState<B> {
    /// Returns the buffer visiting the file at PATH, if one is.
    pub(crate) fn buffer_visiting(&self, path: &str) -> Option<String> {
        self.buffer_names().into_iter().find(|name| {
            self.with_buffer(name, |buf| buf.file_path.as_deref() == Some(path))
                .unwrap_or(false)
        })
    }

    /// Removes the markers of each of PLACED from the buffer visiting FILE.
    fn remove_finding_markers(&self, file: &str, placed: &[Placed]) {
        if placed.is_empty() {
            return;
        }
        let Some(name) = self.buffer_visiting(file) else {
            return;
        };
        self.with_buffer_mut(&name, |buf| {
            for id in placed.iter().flat_map(Placed::ids) {
                buf.markers.remove(id);
            }
        });
    }

    /// Puts FINDINGS in place of what PRODUCER said about FILE, an absolute
    /// path, and makes their markers when a buffer visits it.
    ///
    /// The findings' lock and the buffer's are taken one after the other,
    /// never together: a replacement that comes in between makes the
    /// markers made here not its own, and they go again.
    pub(crate) fn set_findings(&self, producer: &str, file: &str, findings: Vec<Finding>) {
        let replaced = self.findings_mut(|all| all.replace(producer, file, findings.clone()));
        let Some(name) = self.buffer_visiting(file) else {
            return;
        };
        let made = self.with_buffer_mut(&name, |buf| {
            for id in replaced.removed.iter().flat_map(Placed::ids) {
                buf.markers.remove(id);
            }
            replaced.generation.map(|_| make_markers(buf, &findings))
        });
        if let (Some(generation), Some(Some(placed))) = (replaced.generation, made)
            && let Err(placed) =
                self.findings_mut(|all| all.place(producer, file, generation, placed))
        {
            self.remove_finding_markers(file, &[placed]);
        }
    }

    /// Forgets what PRODUCER said about FILE, or about every file when FILE
    /// is `None`, with the markers that followed it.
    pub(crate) fn clear_findings(&self, producer: &str, file: Option<&str>) {
        let removed = self.findings_mut(|all| all.clear(producer, file));
        for (file, placed) in removed {
            self.remove_finding_markers(&file, &[placed]);
        }
    }

    /// Makes the markers of the findings about the file the buffer NAME has
    /// just opened.
    pub(crate) fn place_findings(&self, name: &str) {
        let Some(file) = self
            .with_buffer(name, |buf| buf.file_path.clone())
            .flatten()
        else {
            return;
        };
        let sets = self.findings(|all| all.for_file(&file));
        if sets.is_empty() {
            return;
        }
        let made: Vec<(String, u64, Placed)> = self
            .with_buffer_mut(name, |buf| {
                sets.into_iter()
                    .map(|found| {
                        let placed = make_markers(buf, &found.set.findings);
                        (found.producer, found.set.generation, placed)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let refused: Vec<Placed> = made
            .into_iter()
            .filter_map(|(producer, generation, placed)| {
                self.findings_mut(|all| all.place(&producer, &file, generation, placed))
                    .err()
            })
            .collect();
        if !refused.is_empty() {
            self.with_buffer_mut(name, |buf| {
                for id in refused.iter().flat_map(Placed::ids) {
                    buf.markers.remove(id);
                }
            });
        }
    }

    /// Forgets the markers of the findings about the file the buffer NAME,
    /// which is closing, visits: from now on they are where their producers
    /// said.
    pub(crate) fn unplace_findings(&self, name: &str) {
        let Some((file, version)) = self
            .with_buffer(name, |buf| {
                buf.file_path.clone().map(|file| (file, buf.version))
            })
            .flatten()
        else {
            return;
        };
        self.findings_mut(|all| all.unplace(&file, version));
    }

    /// Returns each finding of SETS where it is now: at its markers in the
    /// buffer visiting its file, when they are there, and where its producer
    /// said otherwise.
    pub(crate) fn locate_findings(&self, sets: Vec<Found>) -> Vec<Located> {
        let mut located = Vec::new();
        for found in sets {
            let Found {
                file,
                producer,
                set,
            } = found;
            let followed = set.placed.as_ref().and_then(|placed| {
                let name = self.buffer_visiting(&file)?;
                self.with_buffer(&name, |buf| {
                    let places: Option<Vec<(LineColumn, LineColumn)>> = placed
                        .markers
                        .iter()
                        .map(|&(start, end)| {
                            Some((
                                line_column_of(&buf.text, buf.markers.get(start)?),
                                line_column_of(&buf.text, buf.markers.get(end)?),
                            ))
                        })
                        .collect();
                    places.map(|places| (places, buf.version != placed.version))
                })
                .flatten()
            });
            let (places, stale) = match followed {
                Some((places, edited)) => (Some(places), edited || set.edited),
                None => (None, set.edited),
            };
            for (at, mut finding) in set.findings.into_iter().enumerate() {
                if let Some(&(start, end)) = places.as_ref().and_then(|places| places.get(at)) {
                    finding.start = start;
                    finding.end = end;
                }
                located.push(Located {
                    file: file.clone(),
                    producer: producer.clone(),
                    finding,
                    stale,
                });
            }
        }
        located
    }
}
