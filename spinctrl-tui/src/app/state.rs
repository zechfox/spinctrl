#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    Monitoring,
    Editing,
    Help,
    Popup(PopupType),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupType {
    ConfirmExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Status = 0,
    Battery = 1,
    CPU = 2,
    Thermal = 3,
    Events = 4,
}

impl Tab {
    const TITLES: &'static [&'static str] = &["Status", "Battery", "CPU", "Thermal", "Events"];
    pub const COUNT: usize = 5;

    pub const fn titles() -> &'static [&'static str] {
        Self::TITLES
    }

    pub const fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Battery,
            2 => Self::CPU,
            3 => Self::Thermal,
            4 => Self::Events,
            _ => Self::Status,
        }
    }

    pub const fn to_index(self) -> usize {
        self as usize
    }

    pub const fn next(self) -> Self {
        Self::from_index((self.to_index() + 1) % Self::COUNT)
    }

    pub const fn previous(self) -> Self {
        Self::from_index((self.to_index() + Self::COUNT - 1) % Self::COUNT)
    }
}