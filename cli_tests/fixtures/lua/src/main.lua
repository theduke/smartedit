local M = {}

--- Create a greeting for a person.
function M.greet(name, punctuation)
  local message = "Hello, " .. name
  return message .. (punctuation or "!")
end

local function calculate(value)
  local function double(number)
    return number * 2
  end

  return double(value) + 1
end

function M.empty() end

local ignored = function() return "anonymous" end

return M
