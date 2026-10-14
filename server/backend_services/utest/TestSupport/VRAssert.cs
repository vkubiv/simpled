using core.Libs.MVCValidation;

namespace utest.TestSupport
{
    public static class VRAssert
    {
        public static void HasErrorCode(ValidationResult vr, string code)
        {
            var errors = vr.GetErrors();

            Assert.Contains(errors, e => e.Code == code);
        }

        public static void HasError(ValidationResult vr, string code, string valueName)
        {
            var errors = vr.GetErrors();

            Assert.Contains(errors, e => e.Code == code && e.IncorrectValue == valueName);
        }

        public static void HasNoErrors(ValidationResult vr)
        {
            var errors = vr.GetErrors();
            Assert.True(errors.Count == 0);
        }
    }
}
