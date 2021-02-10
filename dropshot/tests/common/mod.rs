// Copyright 2020 Oxide Computer Company
/*!
 * Common facilities for automated testing.
 */

use dropshot::test_util::LogContext;
use dropshot::test_util::TestContext;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingIfExists;
use dropshot::ConfigLoggingLevel;

pub fn test_setup(
    test_name: &str,
    api: ApiDescription<usize>,
) -> TestContext<usize> {
    /*
     * The IP address to which we bind can be any local IP, but we use
     * 127.0.0.1 because we know it's present, it shouldn't expose this server
     * on any external network, and we don't have to go looking for some other
     * local IP (likely in a platform-specific way).  We specify port 0 to
     * request any available port.  This is important because we may run
     * multiple concurrent tests, so any fixed port could result in spurious
     * failures due to port conflicts.
     */
    let config_dropshot: ConfigDropshot = Default::default();

    let config_logging = ConfigLogging::File {
        level: ConfigLoggingLevel::Debug,
        path: "UNUSED".to_string(),
        if_exists: ConfigLoggingIfExists::Fail,
    };

    let logctx = LogContext::new(test_name, &config_logging);
    let log = logctx.log.new(o!());
    TestContext::new(api, 0 as usize, &config_dropshot, Some(logctx), log)
}
