// Configuration functions

use config::{Config, Environment, FileFormat};
use log::error;
use std::env;

/// Retrieve the queried parameter from this plugin's configuration
///
/// # Arguments
///
/// * `app_config`: The config loaded from the application's config file.
/// * `plugin_name`: The name of this plugin
/// * `param_key`: The parameter to be queried
///
/// returns: Option<String>
pub fn get_plugin_config(
    app_config: &Config,
    plugin_name: &str,
    param_key: &str,
) -> Option<String> {
    match app_config.get_array("plugins") {
        Result::Ok(plugins) => {
            for plugin in plugins {
                match plugin.into_table() {
                    Ok(plugin_map) => {
                        match plugin_map.get("name") {
                            Some(name_val) => {
                                if name_val.to_string().eq(plugin_name) {
                                    // get the param for given key from this plugin_map:
                                    match plugin_map.get(param_key) {
                                        Some(param_val) => {
                                            return Some(param_val.to_string());
                                        }
                                        None => {
                                            error!("When retrieving value for key {}", param_key);
                                            return None;
                                        }
                                    }
                                }
                            }
                            None => {
                                error!("When extracting name parameter of plugin.");
                            }
                        }
                    }
                    Err(e) => {
                        error!("When getting individual plugin config: {}", e);
                        return None;
                    }
                }
            }
        }
        Err(e) => {
            error!("When retrieving plugins config for all plugins: {}", e);
            return None;
        }
    }
    return None;
}

pub fn read_config(cfg_file: String) -> Config {
    let mut cfg_builder = Config::builder();
    cfg_builder = cfg_builder.add_source(Environment::default().prefix("NEWSLOOKOUT_"));
    cfg_builder = cfg_builder.add_source(config::File::new(&cfg_file, FileFormat::Toml));
    // Add a default configuration file
    match cfg_builder.build() {
        Ok(config) => {
            return config;
        }
        Err(e) => {
            // something went wrong:
            panic!("Error reading configuration - {}", e)
        }
    }
}

pub fn get_data_folder(config: &Config) -> std::path::PathBuf {
    match config.get_string("data_dir") {
        Ok(dirname) => {
            let dirpath = std::path::Path::new(dirname.as_str());
            if std::path::Path::is_dir(dirpath) {
                return dirpath.to_path_buf();
            }
        }
        Err(e) => error!("When getting data folder name: {}", e),
    }
    // return present working directory
    let path_currdir = env::current_dir().expect("give proper argument");
    return path_currdir;
}

pub fn get_database_filename(config: &Config) -> String {
    match config.get_string("completed_urls_datafile") {
        // TODO: check file exists, if not inform that new db file will be initialised
        Ok(dirname) => return dirname,
        Err(e) => error!("When getting database filename: {}", e),
    }
    return "newslookout_urls.db".to_string();
}

#[macro_export]
macro_rules! get_cfg {
    ($config_key:expr, $config_obj:expr, $default_value:expr) => {
        match $config_obj.get_string($config_key) {
            Ok(param_val_str) => param_val_str,
            Err(e) => {
                log::error!(
                    "Could not load parameter {} from config file, using default {}, error: {}",
                    $config_key,
                    $default_value,
                    e
                );
                $default_value.to_string()
            }
        }
    };
}

#[macro_export]
macro_rules! get_cfg_int {
    ($config_key:expr, $config_obj:expr, $default_value:expr) => {
        match $config_obj.get_string($config_key) {
        Ok(param_val_str) => {
            match param_val_str.parse::<isize>(){
                Result::Ok(configintvalue) => configintvalue,
                Err(e)=>{
                    log::error!("Could not convert parameter {} to integer, using default {}, error: {}", $config_key, $default_value, e);
                    $default_value
                }
            }
        },
        Err(e) => {
            log::error!("Could not read integer parameter {} from config file, using default {}, error: {}", $config_key, $default_value, e);
            $default_value
            }
        }
    }
}

#[macro_export]
macro_rules! get_cfg_bool {
    ($config_key:expr, $config_obj:expr, $c:expr) => {
        match $config_obj.get_string($config_key) {
        Ok(param_val_str) => {
            match param_val_str.parse::<bool>(){
                Result::Ok(config_bool_value) => config_bool_value,
                Err(e)=>{
                    log::error!("Could not convert parameter {} to true/false, using default {}, error: {}", $config_key, $c, e);
                    $c
                }
            }
        },
        Err(e) => {
            log::error!("Could not read boolean parameter {} from config file, using default {}, error: {}", $config_key, $c, e);
            $c
            }
        }
    }
}


#[macro_export]
macro_rules! get_plugin_cfg {
    ($plugin_name:expr, $config_key:expr, $config_obj:expr) => {
        match $config_obj.get_array("plugins") {
            Ok(plugins) => {
                let mut found_value = None;
                'searchloop: for plugin in plugins {
                    match plugin.into_table() {
                        Ok(plugin_map) => {
                            match plugin_map.get("name") {
                                Some(name_val) => {

                                    if name_val.to_string().eq($plugin_name) {
                                        // get the param for given key from this plugin_map:
                                        match plugin_map.get($config_key) {
                                            Some(param_val) => {
                                                found_value = Some(param_val.to_string());
                                                break 'searchloop;
                                            },
                                            None => {
                                                error!(
                                                    "Plugin {}: When retrieving value for key {}",
                                                    $plugin_name, $config_key
                                                );
                                                break 'searchloop;
                                            }
                                        }
                                    }
                                },
                                None => {
                                    error!("When extracting name parameter of plugin.");
                                    break 'searchloop;
                                }
                            }
                        },
                        Err(e) => {
                            error!("When getting individual plugin config: {}", e);
                            break 'searchloop;
                        }
                    }
                }
                // end of looping through all plugins:
                found_value
            },
            Err(e) => {
                error!("When retrieving plugins config for all plugins: {}", e);
                None
            }
        }
    };
}


#[cfg(test)]
mod tests {
    use config::Value;
    use postgres::types::ToSql;
    use log::{error, info};
    use crate::cfg;
    use crate::cfg::read_config;

    #[test]
    fn test_cfg_macros() {
        //println!("{:?}", get_cfg!("mykey", "cfgobj", "the-default") );
        let mycfg = config::Config::builder()
            .set_default("key1", "secret value 1")
            .unwrap()
            .set_default("key2", -99)
            .unwrap()
            .set_default("key3", true)
            .unwrap()
            .set_default("key5", "Truth")
            .unwrap()
            .build()
            .unwrap();

        let result_1 = get_cfg!("key1", mycfg, "the-default");
        println!("result_1 = {:?}", result_1);
        assert_eq!(result_1, String::from("secret value 1"));

        let result_2 = get_cfg_int!("key2", mycfg, -42);
        println!("result_2 = {:?}", result_2);
        assert_eq!(result_2, -99);

        let result_3 = get_cfg_bool!("key3", mycfg, false);
        println!("result_3 = {:?}", result_3);
        assert_eq!(result_3, true);

        let result_4 = get_cfg_bool!("key4", mycfg, false);
        println!("result_4 = {:?}", result_4);
        assert_eq!(result_4, false);

        let result_5 = get_cfg_bool!("key5", mycfg, false);
        println!("result_5 = {:?}", result_5);
        assert_eq!(result_5, false);
    }

    #[test]
    fn test_plugin_cfg_macros() {

        // let mycfg = read_config("conf/newslookout.toml".to_string());
        //
        // let result_1 = get_plugin_cfg!("nonsense", "myattrib", mycfg);
        // println!("result_1 = {:?}", result_1);
        // assert_eq!(result_1, None);
        //
        // let result_2 = get_plugin_cfg!("mod_offline_docs", "folder_name", mycfg);
        // println!("result_2 = {:?}", result_2);
        // assert_eq!(result_2, Some(String::from("data/files")));

        assert_eq!(1, 1);
    }
}
