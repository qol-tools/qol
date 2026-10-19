use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Spot {
    pub folder: PathBuf,
    pub cursor: usize,
    pub scroll: f32,
}

impl Spot {
    pub fn at(folder: PathBuf) -> Self {
        Self {
            folder,
            cursor: 0,
            scroll: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct History {
    spots: Vec<Spot>,
    index: usize,
}

impl History {
    pub fn new(start: PathBuf) -> Self {
        Self {
            spots: vec![Spot::at(start)],
            index: 0,
        }
    }

    pub fn current(&self) -> &Spot {
        &self.spots[self.index]
    }

    pub fn remember(&mut self, cursor: usize, scroll: f32) {
        let spot = &mut self.spots[self.index];
        spot.cursor = cursor;
        spot.scroll = scroll;
    }

    pub fn go(&mut self, folder: PathBuf) -> &Spot {
        if self.current().folder == folder {
            return self.current();
        }
        let spot = self.recalled(folder);
        self.spots.truncate(self.index + 1);
        self.spots.push(spot);
        self.index = self.spots.len() - 1;
        self.current()
    }

    pub fn back(&mut self) -> Option<&Spot> {
        self.index = self.index.checked_sub(1)?;
        Some(self.current())
    }

    pub fn forward(&mut self) -> Option<&Spot> {
        if self.index + 1 >= self.spots.len() {
            return None;
        }
        self.index += 1;
        Some(self.current())
    }

    pub fn can_go_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.index + 1 < self.spots.len()
    }

    fn recalled(&self, folder: PathBuf) -> Spot {
        self.spots
            .iter()
            .rev()
            .find(|spot| spot.folder == folder)
            .cloned()
            .unwrap_or_else(|| Spot::at(folder))
    }
}
