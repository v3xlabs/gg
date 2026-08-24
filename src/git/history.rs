use gix::ObjectId;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// One commit in the graph, placed. `incoming` and `outgoing` are lane indices in the gap
/// above and the gap below this row; `through` are lanes that cross the row without
/// touching the commit. A renderer needs nothing else to draw it.
#[derive(Clone)]
pub struct GraphRow {
    pub commit: ObjectId,
    pub lane: usize,
    pub incoming: Vec<usize>,
    pub outgoing: Vec<usize>,
    pub through: Vec<usize>,
}

struct Commit {
    id: ObjectId,
    parents: Vec<ObjectId>,
    time: i64,
}

#[derive(Debug)]
pub enum Error {
    References(Box<dyn std::error::Error + Send + Sync>),
    Walk(Box<dyn std::error::Error + Send + Sync>),
}

pub fn walk(repository: &gix::Repository, limit: usize) -> Result<Vec<GraphRow>, Error> {
    let stashes: HashSet<ObjectId> = crate::git::read::stashes(repository)
        .into_iter()
        .map(|stash| stash.target)
        .collect();

    let tips = commit_tips(repository, &stashes)?;
    if tips.is_empty() {
        return Ok(Vec::new());
    }

    let walk = repository
        .rev_walk(tips)
        .sorting(gix::revision::walk::Sorting::ByCommitTime(
            gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
        ))
        .all()
        .map_err(|error| Error::Walk(Box::new(error)))?;

    let mut commits = Vec::new();
    let mut hidden = HashSet::new();
    for info in walk.take(limit) {
        let info = info.map_err(|error| Error::Walk(Box::new(error)))?;
        let mut parents: Vec<ObjectId> = info.parent_ids().map(|id| id.detach()).collect();

        // A stash is one thing to the reader and three commits to git: the work tree, the
        // index it was taken with, and any untracked files. Only the first parent is
        // history.
        if stashes.contains(&info.id) && parents.len() > 1 {
            hidden.extend(parents.drain(1..));
        }

        commits.push(Commit {
            id: info.id,
            parents,
            time: info.commit_time.unwrap_or_default(),
        });
    }
    commits.retain(|commit| !hidden.contains(&commit.id));

    Ok(assign_lanes(date_topological_order(commits)))
}

fn commit_tips(
    repository: &gix::Repository,
    stashes: &HashSet<ObjectId>,
) -> Result<Vec<ObjectId>, Error> {
    let platform = repository
        .references()
        .map_err(|error| Error::References(Box::new(error)))?;
    let all = platform
        .all()
        .map_err(|error| Error::References(Box::new(error)))?;

    let mut tips = Vec::new();
    for reference in all.filter_map(Result::ok) {
        let Ok(id) = reference.into_fully_peeled_id() else {
            continue;
        };
        if id.object().is_ok_and(|object| object.kind.is_commit()) {
            tips.push(id.detach());
        }
    }

    // refs/stash names only the newest stash; the rest live in its reflog and would
    // otherwise be unreachable from any ref.
    tips.extend(stashes.iter().copied());

    tips.sort_unstable();
    tips.dedup();
    Ok(tips)
}

/// Newest first, but never a parent before one of its children. Sorting by commit time
/// alone is not enough: commits made in the same second tie, and a tie that resolves the
/// wrong way puts a parent above its own child, which makes lane assignment nonsense.
/// This is what `git log --date-order` does.
fn date_topological_order(commits: Vec<Commit>) -> Vec<(ObjectId, Vec<ObjectId>)> {
    let position: HashMap<ObjectId, usize> = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.id, index))
        .collect();

    let mut pending_children = vec![0usize; commits.len()];
    for commit in &commits {
        for parent in &commit.parents {
            if let Some(index) = position.get(parent) {
                pending_children[*index] += 1;
            }
        }
    }

    let mut ready: BinaryHeap<(i64, ObjectId, usize)> = commits
        .iter()
        .enumerate()
        .filter(|(index, _)| pending_children[*index] == 0)
        .map(|(index, commit)| (commit.time, commit.id, index))
        .collect();

    let mut ordered = Vec::with_capacity(commits.len());
    while let Some((_, _, index)) = ready.pop() {
        let commit = &commits[index];
        ordered.push((commit.id, commit.parents.clone()));

        for parent in &commit.parents {
            let Some(parent_index) = position.get(parent).copied() else {
                continue;
            };
            pending_children[parent_index] -= 1;
            if pending_children[parent_index] == 0 {
                let parent = &commits[parent_index];
                ready.push((parent.time, parent.id, parent_index));
            }
        }
    }

    ordered
}

/// Lanes are assigned in one forward pass over the walk order. A commit takes the lane of
/// the first child still waiting for it, its first parent inherits that lane, and every
/// further parent opens or joins another. One pass sometimes crosses more lines than a
/// whole-graph solver would.
pub fn assign_lanes(commits: impl IntoIterator<Item = (ObjectId, Vec<ObjectId>)>) -> Vec<GraphRow> {
    let mut active: Vec<Option<ObjectId>> = Vec::new();
    let mut rows = Vec::new();

    for (commit, parents) in commits {
        let incoming: Vec<usize> = active
            .iter()
            .enumerate()
            .filter(|(_, slot)| **slot == Some(commit))
            .map(|(index, _)| index)
            .collect();

        for index in &incoming {
            active[*index] = None;
        }

        let lane = match incoming.first() {
            Some(index) => *index,
            None => free_lane(&mut active),
        };

        let through: Vec<usize> = active
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.is_some())
            .map(|(index, _)| index)
            .collect();

        let mut outgoing = Vec::with_capacity(parents.len());
        for (position, parent) in parents.iter().enumerate() {
            let target = if position == 0 {
                lane
            } else if let Some(joined) = active.iter().position(|slot| *slot == Some(*parent)) {
                joined
            } else {
                free_lane(&mut active)
            };

            active[target] = Some(*parent);
            outgoing.push(target);
        }

        while active.last().is_some_and(Option::is_none) {
            active.pop();
        }

        rows.push(GraphRow {
            commit,
            lane,
            incoming,
            outgoing,
            through,
        });
    }

    rows
}

fn free_lane(active: &mut Vec<Option<ObjectId>>) -> usize {
    match active.iter().position(Option::is_none) {
        Some(index) => index,
        None => {
            active.push(None);
            active.len() - 1
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::References(error) => write!(formatter, "could not read the references: {error}"),
            Self::Walk(error) => write!(formatter, "could not walk the history: {error}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> ObjectId {
        ObjectId::from_hex(format!("{byte:040x}").as_bytes()).expect("a valid object id")
    }

    fn commit(byte: u8, parents: &[u8], time: i64) -> Commit {
        Commit {
            id: id(byte),
            parents: parents.iter().copied().map(id).collect(),
            time,
        }
    }

    #[test]
    fn a_linear_history_stays_in_one_lane() {
        let rows = assign_lanes([(id(3), vec![id(2)]), (id(2), vec![id(1)]), (id(1), vec![])]);

        assert_eq!(
            rows.iter().map(|row| row.lane).collect::<Vec<_>>(),
            [0, 0, 0]
        );
        assert!(rows.iter().all(|row| row.through.is_empty()));
        assert_eq!(rows[0].incoming, Vec::<usize>::new());
        assert_eq!(rows[0].outgoing, [0]);
        assert_eq!(rows[2].incoming, [0]);
        assert_eq!(rows[2].outgoing, Vec::<usize>::new());
    }

    #[test]
    fn a_merge_opens_a_lane_and_the_join_closes_it() {
        let rows = assign_lanes([
            (id(4), vec![id(3), id(2)]),
            (id(3), vec![id(1)]),
            (id(2), vec![id(1)]),
            (id(1), vec![]),
        ]);

        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[0].outgoing, [0, 1], "the second parent opens lane 1");

        assert_eq!(rows[1].lane, 0);
        assert_eq!(rows[1].through, [1], "lane 1 crosses this row untouched");

        assert_eq!(rows[2].lane, 1);
        assert_eq!(
            rows[2].outgoing,
            [1],
            "a first parent always inherits its child's lane, which keeps trunks straight"
        );

        assert_eq!(rows[3].lane, 0);
        assert_eq!(
            rows[3].incoming,
            [0, 1],
            "both lanes reached the base, so both converge on it"
        );
    }

    #[test]
    fn a_lane_is_reused_once_its_last_child_is_placed() {
        let rows = assign_lanes([
            (id(5), vec![id(3)]),
            (id(4), vec![id(3)]),
            (id(3), vec![]),
            (id(2), vec![]),
        ]);

        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1, "a second tip needs a second lane");
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[2].incoming, [0, 1], "both children converge here");
        assert_eq!(rows[3].lane, 0, "every lane is free again");
    }

    #[test]
    fn equal_timestamps_never_put_a_parent_above_its_child() {
        // The shape the .tmp fixture builds, every commit in the same second.
        let ordered = date_topological_order(vec![
            commit(1, &[], 100),
            commit(2, &[1], 100),
            commit(3, &[2], 100),    // C on main
            commit(4, &[2], 100),    // F1
            commit(5, &[4], 100),    // F2
            commit(6, &[3, 5], 100), // merge feature
            commit(7, &[2], 100),    // O1 on other
            commit(8, &[6, 7], 100), // merge other
            commit(9, &[8], 100),    // D
        ]);

        let seen_at: HashMap<ObjectId, usize> = ordered
            .iter()
            .enumerate()
            .map(|(index, (id, _))| (*id, index))
            .collect();

        for (child, parents) in &ordered {
            for parent in parents {
                assert!(
                    seen_at[child] < seen_at[parent],
                    "a parent was emitted before its child"
                );
            }
        }

        assert_eq!(ordered[0].0, id(9), "the tip comes first");
    }

    #[test]
    fn newer_commits_come_first_when_topology_allows() {
        let ordered = date_topological_order(vec![
            commit(1, &[], 10),
            commit(2, &[1], 20),
            commit(3, &[1], 30),
        ]);

        assert_eq!(
            ordered.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [id(3), id(2), id(1)]
        );
    }
}
