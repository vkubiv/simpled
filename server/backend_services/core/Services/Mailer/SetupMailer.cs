using config;

namespace core.Services.Mailer;

public static class SetupMailer
{
    
    public static IMailer CreateMailer(CommonConfiguration config, string templatesDirectoryPath)
    {
        var langs = new[] { "en" };
        var templates = MainTemplater.LoadFromDiskStorage(templatesDirectoryPath, langs).Result;
        
        return CreateMailer(config, templates);
    }
    
    public static IMailer CreateMailer(CommonConfiguration config,  MainTemplater templates)
    {
        var sendGridMailerOpts = new SendGridMailerOpts
        {
            ApiKey = config.SendGridApiKey,
            Host = config.SendGridApiHost,
        };

        IMailer sendGridMailer = new SendGridMailer(sendGridMailerOpts, templates);

        if (config.SendGridMockApiHost == null)
        {
            return sendGridMailer;
        }

        var mockSendGridMailerOpts = new SendGridMailerOpts
        {
            ApiKey = config.SendGridMockApiKey,
            Host = config.SendGridMockApiHost,
        };

        return new RealMockCombinedMailer(sendGridMailer, new SendGridMailer(mockSendGridMailerOpts, templates));
    }
}