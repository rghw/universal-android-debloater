use crate::core::sync::{action_handler, perform_adb_commands, CommandType, Phone, User};
use crate::core::theme::Theme;
use crate::core::uad_lists::{
    load_debloat_lists, Opposite, Package, PackageState, Removal, UadList, UadListState,
};
use crate::core::utils::{fetch_packages, update_selection_count};
use crate::gui::style;
use std::collections::HashMap;
use std::env;

use crate::gui::views::settings::Settings;
use crate::gui::widgets::package_row::{Message as RowMessage, PackageRow};
use iced::widget::{
    button, column, container, pick_list, row, scrollable, text, text_input, Space,
};
use iced::{Alignment, Command, Element, Length, Renderer};

#[derive(Debug, Default, Clone)]
pub struct Selection {
    pub uninstalled: u16,
    pub enabled: u16,
    pub disabled: u16,
    pub selected_packages: Vec<usize>, // phone_packages indexes (= what you've selected)
}

#[derive(Debug, Default, Clone)]
pub struct PackageInfo {
    pub i_user: Option<usize>,
    pub index: usize,
    pub removal: String,
}

#[derive(Debug, Clone)]
pub enum Action {
    Remove,
    Restore,
}

#[derive(Debug, Clone)]
pub enum LoadingState {
    DownloadingList(String),
    FindingPhones(String),
    LoadingPackages(String),
    _UpdatingUad(String),
    Ready(String),
    RestoringDevice(String),
}

impl Default for LoadingState {
    fn default() -> Self {
        Self::FindingPhones("".to_string())
    }
}

#[derive(Default, Debug, Clone)]
pub struct List {
    pub loading_state: LoadingState,
    pub uad_lists: HashMap<String, Package>,
    pub phone_packages: Vec<Vec<PackageRow>>, // packages of all users of the phone
    filtered_packages: Vec<usize>, // phone_packages indexes of the selected user (= what you see on screen)
    pub selection: Selection,
    selected_package_state: Option<PackageState>,
    selected_removal: Option<Removal>,
    selected_list: Option<UadList>,
    selected_user: Option<User>,
    pub input_value: String,
    description: String,
    current_package_index: usize,
}

#[derive(Debug, Clone)]
pub enum Message {
    LoadUadList(bool),
    LoadPhonePackages((HashMap<String, Package>, UadListState)),
    RestoringDevice(Result<CommandType, ()>),
    ApplyFilters(Vec<Vec<PackageRow>>),
    SearchInputChanged(String),
    ToggleAllSelected(bool),
    ListSelected(UadList),
    UserSelected(User),
    PackageStateSelected(PackageState),
    RemovalSelected(Removal),
    ApplyActionOnSelection(Action),
    List(usize, RowMessage),
    ChangePackageState(Result<CommandType, ()>),
    Nothing,
}

impl List {
    pub fn update(
        &mut self,
        settings: &mut Settings,
        selected_device: &mut Phone,
        list_update_state: &mut UadListState,
        message: Message,
    ) -> Command<Message> {
        let i_user = self.selected_user.unwrap_or(User { id: 0, index: 0 }).index;
        match message {
            Message::RestoringDevice(output) => {
                if let Ok(res) = output {
                    if let CommandType::PackageManager(p) = res {
                        self.loading_state = LoadingState::RestoringDevice(
                            self.phone_packages[i_user][p.index].name.clone(),
                        )
                    }
                } else {
                    self.loading_state = LoadingState::RestoringDevice("Error [TODO]".to_string());
                }
                Command::none()
            }
            Message::LoadUadList(remote) => {
                info!("{:-^65}", "-");
                info!(
                    "ANDROID_SDK: {} | DEVICE: {}",
                    selected_device.android_sdk, selected_device.model
                );
                info!("{:-^65}", "-");
                self.loading_state = LoadingState::DownloadingList("".to_string());
                Command::perform(
                    Self::init_apps_view(remote, selected_device.clone()),
                    Message::LoadPhonePackages,
                )
            }
            Message::LoadPhonePackages(list_box) => {
                let (uad_list, list_state) = list_box;
                self.loading_state = LoadingState::LoadingPackages("".to_string());
                self.uad_lists = uad_list.clone();
                *list_update_state = list_state;
                Command::perform(
                    Self::load_packages(uad_list, selected_device.user_list.clone()),
                    Message::ApplyFilters,
                )
            }
            Message::ApplyFilters(packages) => {
                self.phone_packages = packages;
                self.filtered_packages = (0..self.phone_packages[i_user].len()).collect();
                self.selected_package_state = Some(PackageState::Enabled);
                self.selected_removal = Some(Removal::Recommended);
                self.selected_list = Some(UadList::All);
                self.selected_user = Some(User { id: 0, index: 0 });
                Self::filter_package_lists(self);
                self.loading_state = LoadingState::Ready("".to_string());
                Command::none()
            }
            Message::ToggleAllSelected(selected) => {
                for i in self.filtered_packages.clone() {
                    self.phone_packages[i_user][i].selected = selected;

                    if !selected {
                        if self.selection.selected_packages.contains(&i) {
                            update_selection_count(
                                &mut self.selection,
                                self.phone_packages[i_user][i].state,
                                false,
                            );
                            self.selection
                                .selected_packages
                                .drain_filter(|s_i| *s_i == i);
                        }
                    } else if !self.selection.selected_packages.contains(&i) {
                        self.selection.selected_packages.push(i);
                        update_selection_count(
                            &mut self.selection,
                            self.phone_packages[i_user][i].state,
                            true,
                        );
                    }
                }
                Command::none()
            }
            Message::SearchInputChanged(letter) => {
                self.input_value = letter;
                Self::filter_package_lists(self);
                Command::none()
            }
            Message::ListSelected(list) => {
                self.selected_list = Some(list);
                Self::filter_package_lists(self);
                Command::none()
            }
            Message::PackageStateSelected(package_state) => {
                self.selected_package_state = Some(package_state);
                Self::filter_package_lists(self);
                Command::none()
            }
            Message::RemovalSelected(removal) => {
                self.selected_removal = Some(removal);
                Self::filter_package_lists(self);
                Command::none()
            }
            Message::List(i_package, row_message) => {
                self.phone_packages[i_user][i_package]
                    .update(row_message.clone())
                    .map(move |row_message| Message::List(i_package, row_message));

                let package = &mut self.phone_packages[i_user][i_package];

                match row_message {
                    RowMessage::ToggleSelection(toggle) => {
                        if package.removal == Removal::Unsafe && !settings.general.expert_mode {
                            package.selected = false;
                        } else {
                            package.selected = toggle;

                            if package.selected {
                                self.selection.selected_packages.push(i_package);
                            } else {
                                self.selection
                                    .selected_packages
                                    .drain_filter(|s_i| *s_i == i_package);
                            }
                            update_selection_count(
                                &mut self.selection,
                                package.state,
                                package.selected,
                            );
                        }
                        Command::none()
                    }
                    RowMessage::ActionPressed => {
                        let mut commands = vec![];
                        let actions = action_handler(
                            &self.selected_user.unwrap(),
                            &package.into(),
                            selected_device,
                            &settings.device,
                        );

                        for (i, (i_user, action)) in actions.into_iter().enumerate() {
                            let p_info = PackageInfo {
                                i_user,
                                index: i_package,
                                removal: package.removal.to_string(),
                            };
                            // Only the first command can change the package state
                            commands.push(Command::perform(
                                perform_adb_commands(action, CommandType::PackageManager(p_info)),
                                if i == 0 {
                                    Message::ChangePackageState
                                } else {
                                    |_| Message::Nothing
                                },
                            ));
                        }
                        Command::batch(commands)
                    }
                    RowMessage::PackagePressed => {
                        self.description = package.clone().description;
                        package.current = true;
                        if self.current_package_index != i_package {
                            self.phone_packages[i_user][self.current_package_index].current = false;
                        }
                        self.current_package_index = i_package;
                        Command::none()
                    }
                }
            }
            Message::ApplyActionOnSelection(action) => {
                let mut selected_packages = self.selection.selected_packages.clone();

                match action {
                    Action::Remove => {
                        selected_packages.drain_filter(|i| {
                            self.phone_packages[i_user][*i].state != PackageState::Enabled
                        });
                    }
                    Action::Restore => {
                        selected_packages.drain_filter(|i| {
                            self.phone_packages[i_user][*i].state == PackageState::Enabled
                        });
                    }
                }
                let mut commands = vec![];
                for i in selected_packages {
                    let actions = action_handler(
                        &self.selected_user.unwrap(),
                        &(&self.phone_packages[i_user][i]).into(),
                        selected_device,
                        &settings.device,
                    );

                    let package = &mut self.phone_packages[i_user][i];
                    for (j, (i_user, action)) in actions.into_iter().enumerate() {
                        let p_info = PackageInfo {
                            i_user,
                            index: i,
                            removal: package.removal.to_string(),
                        };
                        // Only the first command can change the package state
                        commands.push(Command::perform(
                            perform_adb_commands(action, CommandType::PackageManager(p_info)),
                            if j == 0 {
                                Message::ChangePackageState
                            } else {
                                |_| Message::Nothing
                            },
                        ));
                    }
                }
                Command::batch(commands)
            }
            Message::UserSelected(user) => {
                for p in &mut self.phone_packages[i_user] {
                    p.selected = false;
                }
                self.selected_user = Some(user);
                for i_package in &self.selection.selected_packages {
                    self.phone_packages[user.index][*i_package].selected = true;
                }
                self.filtered_packages = (0..self.phone_packages[user.index].len()).collect();
                Self::filter_package_lists(self);
                Command::none()
            }
            Message::ChangePackageState(res) => {
                if let Ok(CommandType::PackageManager(p)) = res {
                    let package = &mut self.phone_packages[i_user][p.index];
                    update_selection_count(&mut self.selection, package.state, false);

                    if !settings.device.multi_user_mode || p.i_user.is_none() {
                        package.state = package.state.opposite(settings.device.disable_mode);
                        package.selected = false;
                    } else {
                        self.phone_packages[p.i_user.unwrap()][p.index].state = self.phone_packages
                            [p.i_user.unwrap()][p.index]
                            .state
                            .opposite(settings.device.disable_mode);
                        self.phone_packages[p.i_user.unwrap()][p.index].selected = false;
                    }
                    self.selection
                        .selected_packages
                        .drain_filter(|s_i| *s_i == p.index);
                    Self::filter_package_lists(self);
                }
                Command::none()
            }
            Message::Nothing => Command::none(),
        }
    }

    pub fn view(
        &self,
        settings: &Settings,
        selected_device: &Phone,
    ) -> Element<Message, Renderer<Theme>> {
        match &self.loading_state {
            LoadingState::DownloadingList(_) => {
                let text = "Downloading latest UAD lists from Github. Please wait...";
                waiting_view(settings, text, true)
            }
            LoadingState::FindingPhones(_) => {
                let text = "Finding connected devices...";
                waiting_view(settings, text, false)
            }
            LoadingState::LoadingPackages(_) => {
                let text = "Pulling packages from the device. Please wait...";
                waiting_view(settings, text, false)
            }
            LoadingState::_UpdatingUad(_) => {
                let text = "Updating UAD. Please wait...";
                waiting_view(settings, text, false)
            }
            LoadingState::RestoringDevice(output) => {
                let text = format!("Restoring device: {}", output);
                waiting_view(settings, &text, false)
            }
            LoadingState::Ready(_) => {
                let search_packages = text_input(
                    "Search packages...",
                    &self.input_value,
                    Message::SearchInputChanged,
                )
                .padding(5);

                // let package_amount = text(format!("{} packages found", packages.len()));

                let user_picklist = pick_list(
                    selected_device.user_list.clone(),
                    self.selected_user,
                    Message::UserSelected,
                )
                .width(Length::Units(85));

                let divider = Space::new(Length::Fill, Length::Shrink);

                let list_picklist =
                    pick_list(&UadList::ALL[..], self.selected_list, Message::ListSelected);
                let package_state_picklist = pick_list(
                    &PackageState::ALL[..],
                    self.selected_package_state,
                    Message::PackageStateSelected,
                );

                let removal_picklist = pick_list(
                    &Removal::ALL[..],
                    self.selected_removal,
                    Message::RemovalSelected,
                );

                let control_panel = row![
                    search_packages,
                    user_picklist,
                    divider,
                    removal_picklist,
                    package_state_picklist,
                    list_picklist,
                ]
                .width(Length::Fill)
                .align_items(Alignment::Center)
                .spacing(10)
                .padding([0, 16, 0, 0]);

                let packages =
                    self.filtered_packages
                        .iter()
                        .fold(column![].spacing(6), |col, i| {
                            col.push(
                                self.phone_packages[self.selected_user.unwrap().index][*i]
                                    .view(settings, selected_device)
                                    .map(move |msg| Message::List(*i, msg)),
                            )
                        });

                let packages_scrollable = scrollable(packages)
                    .scrollbar_margin(2)
                    .height(Length::FillPortion(6))
                    .style(style::Scrollable::Packages);

                // let mut packages_v: Vec<&str> = self.packages.lines().collect();

                let description_scroll = scrollable(text(&self.description))
                    .scrollbar_margin(7)
                    .style(style::Scrollable::Description);

                let description_panel = container(description_scroll)
                    .height(Length::FillPortion(2))
                    .width(Length::Fill)
                    .style(style::Container::Frame);

                let restore_action = match settings.device.disable_mode {
                    true => "Enable/Restore",
                    false => "Restore",
                };
                let remove_action = match settings.device.disable_mode {
                    true => "Disable",
                    false => "Uninstall",
                };

                let apply_restore_selection = button(text(format!(
                    "{} selection ({})",
                    restore_action,
                    self.selection.uninstalled + self.selection.disabled
                )))
                .on_press(Message::ApplyActionOnSelection(Action::Restore))
                .padding(5)
                .style(style::Button::Primary);

                let apply_remove_selection = button(text(format!(
                    "{} selection ({})",
                    remove_action, self.selection.enabled
                )))
                .on_press(Message::ApplyActionOnSelection(Action::Remove))
                .padding(5)
                .style(style::Button::Primary);

                let select_all_btn = button("Select all")
                    .padding(5)
                    .on_press(Message::ToggleAllSelected(true))
                    .style(style::Button::Primary);

                let unselect_all_btn = button("Unselect all")
                    .padding(5)
                    .on_press(Message::ToggleAllSelected(false))
                    .style(style::Button::Primary);

                let action_row = row![
                    select_all_btn,
                    unselect_all_btn,
                    Space::new(Length::Fill, Length::Shrink),
                    apply_restore_selection,
                    apply_remove_selection,
                ]
                .width(Length::Fill)
                .spacing(10)
                .align_items(Alignment::Center);

                let content = column![
                    control_panel,
                    packages_scrollable,
                    description_panel,
                    action_row,
                ]
                .width(Length::Fill)
                .spacing(10)
                .align_items(Alignment::Center);

                container(content).height(Length::Fill).padding(10).into()
            }
        }
    }

    fn filter_package_lists(&mut self) {
        let list_filter: UadList = self.selected_list.unwrap();
        let package_filter: PackageState = self.selected_package_state.unwrap();
        let removal_filter: Removal = self.selected_removal.unwrap();

        self.filtered_packages = self.phone_packages[self.selected_user.unwrap().index]
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                (list_filter == UadList::All || p.uad_list == list_filter)
                    && (package_filter == PackageState::All || p.state == package_filter)
                    && (removal_filter == Removal::All || p.removal == removal_filter)
                    && (self.input_value.is_empty() || p.name.contains(&self.input_value))
            })
            .map(|(i, _)| i)
            .collect();
    }

    async fn load_packages(
        uad_list: HashMap<String, Package>,
        user_list: Vec<User>,
    ) -> Vec<Vec<PackageRow>> {
        let mut phone_packages = vec![];

        if user_list.len() <= 1 {
            phone_packages.push(fetch_packages(&uad_list, None))
        } else {
            phone_packages.extend(
                user_list
                    .iter()
                    .map(|user| fetch_packages(&uad_list, Some(user))),
            )
        };
        phone_packages
    }

    async fn init_apps_view(
        remote: bool,
        phone: Phone,
    ) -> (HashMap<String, Package>, UadListState) {
        let (uad_lists, _) = load_debloat_lists(remote);
        match uad_lists {
            Ok(list) => {
                env::set_var("ANDROID_SERIAL", phone.adb_id.clone());
                if phone.adb_id.is_empty() {
                    error!("AppsView ready but no phone found");
                }
                (list, UadListState::Done)
            }
            Err(local_list) => {
                error!("Error loading remote debloat list for the phone. Fallback to embedded (and outdated) list");
                (local_list, UadListState::Failed)
            }
        }
    }
}

fn waiting_view<'a>(
    _settings: &Settings,
    displayed_text: &str,
    btn: bool,
) -> Element<'a, Message, Renderer<Theme>> {
    let col = if btn {
        let no_internet_btn = button("No internet?")
            .padding(5)
            .on_press(Message::LoadUadList(false))
            .style(style::Button::Primary);

        column![]
            .spacing(10)
            .align_items(Alignment::Center)
            .push(text(displayed_text).size(20))
            .push(no_internet_btn)
    } else {
        column![]
            .spacing(10)
            .align_items(Alignment::Center)
            .push(text(displayed_text).size(20))
    };

    container(col)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_y()
        .center_x()
        .style(style::Container::default())
        .into()
}
